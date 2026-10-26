
#![deny(unsafe_code)]

use crate::protocol::{IpcError, IpcMessage, IpcReader, IpcWriter};
use crate::shmem::SharedTripleBuffer;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex as TokioMutex;
use tracing::{error, info, warn};

#[cfg(unix)]
type TabTransport = tokio::net::UnixStream;
#[cfg(windows)]
type TabTransport =
    tokio::net::windows::named_pipe::NamedPipeServer;

type BrowserWriter = tokio::io::WriteHalf<TabTransport>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabStatus {
    Initializing,
    Running { url: String, title: String },
    Crashed { reason: String },
    Closed,
}

pub struct TabProcessHandle {
    pub tab_id: u32,
    status: Arc<StdMutex<TabStatus>>,
    writer: Option<Arc<TokioMutex<IpcWriter<BrowserWriter>>>>,
    heartbeat: Arc<StdMutex<Instant>>,
    pub shmem: Option<SharedTripleBuffer>,
    shmem_path: Option<PathBuf>,
    socket_path: PathBuf,
    child: Option<Arc<TokioMutex<tokio::process::Child>>>,
    initial_url: String,
    inbox: Arc<StdMutex<Vec<IpcMessage>>>,
}

pub struct BrowserProcessManager {
    next_tab_id: u32,
    tabs: HashMap<u32, TabProcessHandle>,
    socket_dir: PathBuf,
    _temp_dir: Option<tempfile::TempDir>,
    generation: u64,
}

impl Default for BrowserProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserProcessManager {
    pub fn new() -> Self {
        let temp_dir = tempfile::Builder::new()
            .prefix("rix-tabs-")
            .tempdir()
            .ok();
        let socket_dir = temp_dir.as_ref().map_or_else(
            || std::env::temp_dir().join(format!("rix-tabs-{}", std::process::id())),
            |d| d.path().to_path_buf(),
        );
        let _ = std::fs::create_dir_all(&socket_dir);
        Self {
            next_tab_id: 1,
            tabs: HashMap::new(),
            socket_dir,
            _temp_dir: temp_dir,
            generation: 0,
        }
    }

    pub fn socket_dir(&self) -> &Path {
        &self.socket_dir
    }

    fn socket_path_for(&self, tab_id: u32, generation: u64) -> PathBuf {
        #[cfg(unix)]
        {
            self.socket_dir
                .join(format!("rix-tab-{}-g{}.sock", tab_id, generation))
        }
        #[cfg(windows)]
        {
            self.socket_dir
                .join(format!("rix-tab-{}-g{}.pipe", tab_id, generation))
        }
    }

    #[cfg(windows)]
    fn pipe_name_for(tab_id: u32, generation: u64) -> String {
        format!(
            r"\\.\pipe\rix-tab-{}-g{}-p{}",
            tab_id,
            generation,
            std::process::id()
        )
    }

    pub fn tab_status(&self, tab_id: u32) -> Option<TabStatus> {
        self.tabs.get(&tab_id).map(|t| {
            t.status
                .lock()
                .map(|s| s.clone())
                .unwrap_or(TabStatus::Crashed {
                    reason: "status lock poisoned".to_string(),
                })
        })
    }

    pub fn mark_tab_running(&mut self, tab_id: u32, url: String, title: String) {
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            if let Ok(mut s) = tab.status.lock() {
                *s = TabStatus::Running { url, title };
            }
        }
    }

    pub fn mark_tab_crashed(&mut self, tab_id: u32, reason: String) {
        warn!("Tab #{} crashed: {}", tab_id, reason);
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            if let Ok(mut s) = tab.status.lock() {
                *s = TabStatus::Crashed { reason };
            }
        }
    }

    pub fn record_heartbeat(&mut self, tab_id: u32) {
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            if let Ok(mut hb) = tab.heartbeat.lock() {
                *hb = Instant::now();
            }
        }
    }

    pub fn set_last_heartbeat_for_test(&mut self, tab_id: u32, instant: Instant) {
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            if let Ok(mut hb) = tab.heartbeat.lock() {
                *hb = instant;
            }
        }
    }

    pub fn shmem(&self, tab_id: u32) -> Option<&SharedTripleBuffer> {
        self.tabs.get(&tab_id).and_then(|t| t.shmem.as_ref())
    }

    pub fn drain_messages(&self, tab_id: u32) -> Vec<IpcMessage> {
        self.tabs
            .get(&tab_id)
            .map(|t| {
                t.inbox
                    .lock()
                    .map(|mut q| std::mem::take(&mut *q))
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    pub async fn wait_for_message<F>(
        &self,
        tab_id: u32,
        timeout: Duration,
        pred: F,
    ) -> Result<IpcMessage, IpcError>
    where
        F: Fn(&IpcMessage) -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            let found = self
                .tabs
                .get(&tab_id)
                .and_then(|t| {
                    t.inbox.lock().ok().and_then(|mut q| {
                        q.iter()
                            .position(|m| pred(m))
                            .map(|pos| q.remove(pos))
                    })
                });
            if let Some(msg) = found {
                return Ok(msg);
            }
            if Instant::now() >= deadline {
                return Err(IpcError::AcceptTimeout);
            }
            if let Some(status) = self.tab_status(tab_id) {
                if matches!(status, TabStatus::Crashed { .. } | TabStatus::Closed) {
                    let pending: bool = self
                        .tabs
                        .get(&tab_id)
                        .and_then(|t| t.inbox.lock().ok().map(|q| q.iter().any(|m| pred(m))))
                        .unwrap_or(false);
                    if !pending {
                        return Err(IpcError::ChildExited(format!(
                            "tab {} is {:?} while waiting for message",
                            tab_id, status
                        )));
                    }
                }
            } else {
                return Err(IpcError::NoSuchTab(tab_id));
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    pub async fn spawn_tab(
        &mut self,
        initial_url: &str,
        width: u32,
        height: u32,
    ) -> Result<u32, IpcError> {
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;
        self.generation += 1;
        let generation = self.generation;

        let socket_path = self.socket_path_for(tab_id, generation);
        let (shmem, shmem_path) =
            SharedTripleBuffer::new_persistent(width, height, 4, None).map_err(|e| {
                IpcError::SpawnFailed(format!("shmem allocation failed: {}", e))
            })?;

        match self
            .spawn_and_accept(tab_id, generation, &socket_path, shmem, shmem_path.clone(), initial_url)
            .await
        {
            Ok(()) => {
                info!("Spawned tab #{} for url '{}'", tab_id, initial_url);
                Ok(tab_id)
            }
            Err(e) => {
                self.tabs.remove(&tab_id);
                let _ = std::fs::remove_file(&socket_path);
                let _ = std::fs::remove_file(&shmem_path);
                Err(e)
            }
        }
    }

    async fn spawn_and_accept(
        &mut self,
        tab_id: u32,
        generation: u64,
        socket_path: &Path,
        shmem: SharedTripleBuffer,
        shmem_path: PathBuf,
        initial_url: &str,
    ) -> Result<(), IpcError> {
        let _ = generation;
        #[cfg(unix)]
        let listener = crate::protocol::bind_unix_listener(socket_path)?;

        #[cfg(windows)]
        let listener = {
            let name = Self::pipe_name_for(tab_id, generation);
            tokio::net::windows::named_pipe::ServerOptions::new()
                .first_pipe_instance(true)
                .create(&name)
                .map_err(IpcError::Io)?
        };

        let exe = Self::tab_worker_exe();
        let mut cmd = tokio::process::Command::new(&exe);
        #[cfg(unix)]
        {
            cmd.arg("--socket")
                .arg(socket_path)
                .arg("--tab-id")
                .arg(tab_id.to_string())
                .arg("--shmem")
                .arg(&shmem_path);
        }
        #[cfg(windows)]
        {
            cmd.arg("--socket")
                .arg(Self::pipe_name_for(tab_id, generation))
                .arg("--tab-id")
                .arg(tab_id.to_string())
                .arg("--shmem")
                .arg(&shmem_path);
        }
        cmd.kill_on_drop(true);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        let child = cmd
            .spawn()
            .map_err(|e| IpcError::SpawnFailed(format!("{}: {}", exe.display(), e)))?;

        #[cfg(unix)]
        let stream: TabTransport = {
            let (stream, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
                .await
                .map_err(|_| IpcError::AcceptTimeout)?
                .map_err(IpcError::Io)?;
            stream
        };
        #[cfg(windows)]
        let stream: TabTransport = {
            tokio::time::timeout(Duration::from_secs(10), listener.connect())
                .await
                .map_err(|_| IpcError::AcceptTimeout)?
                .map_err(IpcError::Io)?;
            listener
        };

        self.install_tab(
            tab_id,
            socket_path.to_path_buf(),
            stream,
            child,
            shmem,
            shmem_path,
            initial_url.to_string(),
        );
        Ok(())
    }

    fn install_tab(
        &mut self,
        tab_id: u32,
        socket_path: PathBuf,
        stream: TabTransport,
        child: tokio::process::Child,
        shmem: SharedTripleBuffer,
        shmem_path: PathBuf,
        initial_url: String,
    ) {
        let (r, w) = tokio::io::split(stream);
        let mut reader = IpcReader::new(r);
        let writer: Option<Arc<TokioMutex<IpcWriter<BrowserWriter>>>> =
            Some(Arc::new(TokioMutex::new(IpcWriter::new(w))));
        let status = Arc::new(StdMutex::new(TabStatus::Initializing));
        let heartbeat = Arc::new(StdMutex::new(Instant::now()));
        let inbox: Arc<StdMutex<Vec<IpcMessage>>> = Arc::new(StdMutex::new(Vec::new()));
        let child = Arc::new(TokioMutex::new(child));

        {
            let status_c = Arc::clone(&status);
            let heartbeat_c = Arc::clone(&heartbeat);
            let inbox_c = Arc::clone(&inbox);
            tokio::spawn(async move {
                loop {
                    match reader.read_message().await {
                        Ok(msg) => {
                            match &msg {
                                IpcMessage::HeartbeatPong { .. } => {
                                    if let Ok(mut hb) = heartbeat_c.lock() {
                                        *hb = Instant::now();
                                    }
                                }
                                IpcMessage::PageLoaded { url, title, .. } => {
                                    if let Ok(mut s) = status_c.lock() {
                                        *s = TabStatus::Running {
                                            url: url.clone(),
                                            title: title.clone(),
                                        };
                                    }
                                    if let Ok(mut hb) = heartbeat_c.lock() {
                                        *hb = Instant::now();
                                    }
                                }
                                IpcMessage::CrashNotification { reason, .. } => {
                                    warn!("Tab #{} reported crash: {}", tab_id, reason);
                                    if let Ok(mut s) = status_c.lock() {
                                        *s = TabStatus::Crashed {
                                            reason: reason.clone(),
                                        };
                                    }
                                }
                                IpcMessage::RenderFrameReady { .. } => {
                                    if let Ok(mut hb) = heartbeat_c.lock() {
                                        *hb = Instant::now();
                                    }
                                }
                                _ => {}
                            }
                            if let Ok(mut q) = inbox_c.lock() {
                                q.push(msg);
                                if q.len() > 4096 {
                                    let excess = q.len() - 4096;
                                    q.drain(0..excess);
                                }
                            }
                        }
                        Err(IpcError::ConnectionClosed) => {
                            warn!("Tab #{} IPC closed by child", tab_id);
                            break;
                        }
                        Err(err) => {
                            error!("IPC read error on tab #{}: {}", tab_id, err);
                            break;
                        }
                    }
                }
            });
        }

        {
            let status_c = Arc::clone(&status);
            let inbox_c = Arc::clone(&inbox);
            let child_c = Arc::clone(&child);
            tokio::spawn(async move {
                loop {
                    let exited: Option<std::process::ExitStatus> = {
                        let mut guard = child_c.lock().await;
                        match guard.try_wait() {
                            Ok(opt) => opt,
                            Err(_) => None,
                        }
                    };
                    if let Some(code) = exited {
                        if code.success() {
                            info!("Tab #{} child exited cleanly: {}", tab_id, code);
                        } else {
                            let reason = format!("Tab process exited with {}", code);
                            warn!("Tab #{}: {}", tab_id, reason);
                            if let Ok(mut s) = status_c.lock() {
                                if !matches!(*s, TabStatus::Closed) {
                                    *s = TabStatus::Crashed {
                                        reason: reason.clone(),
                                    };
                                }
                            }
                            if let Ok(mut q) = inbox_c.lock() {
                                q.push(IpcMessage::CrashNotification {
                                    tab_id,
                                    reason,
                                });
                            }
                        }
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            });
        }

        self.tabs.insert(
            tab_id,
            TabProcessHandle {
                tab_id,
                status,
                writer,
                heartbeat,
                shmem: Some(shmem),
                shmem_path: Some(shmem_path),
                socket_path,
                child: Some(child),
                initial_url,
                inbox,
            },
        );
    }

    pub fn tab_worker_exe() -> PathBuf {
        let exe_name = format!(
            "rix-tab-worker{}",
            std::env::consts::EXE_SUFFIX
        );
        if let Ok(p) = std::env::var("RIX_TAB_WORKER_EXE") {
            let pb = PathBuf::from(&p);
            if pb.exists() {
                return pb;
            }
        }
        if let Ok(p) = std::env::var("CARGO_BIN_EXE_rix-tab-worker") {
            let pb = PathBuf::from(&p);
            if pb.exists() {
                return pb;
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut dir = exe.parent().map(Path::to_path_buf);
            for _ in 0..5 {
                if let Some(d) = dir.clone() {
                    let cand = d.join(&exe_name);
                    if cand.exists() {
                        return cand;
                    }
                    dir = d.parent().map(Path::to_path_buf);
                } else {
                    break;
                }
            }
        }
        for profile in ["debug", "release"] {
            let cand = PathBuf::from("target").join(profile).join(&exe_name);
            if cand.exists() {
                return cand;
            }
        }
        PathBuf::from(exe_name)
    }

    pub async fn reload_crashed_tab(
        &mut self,
        tab_id: u32,
        width: u32,
        height: u32,
    ) -> Result<(), IpcError> {
        let (initial_url, old_socket, old_shmem_path) = {
            let tab = self.tabs.get(&tab_id).ok_or(IpcError::NoSuchTab(tab_id))?;
            (
                tab.initial_url.clone(),
                tab.socket_path.clone(),
                tab.shmem_path.clone(),
            )
        };

        if let Some(tab) = self.tabs.get(&tab_id) {
            if let Some(child) = &tab.child {
                let mut guard = child.lock().await;
                let _ = guard.kill().await;
            }
        }

        self.generation += 1;
        let generation = self.generation;
        let socket_path = self.socket_path_for(tab_id, generation);
        let _ = std::fs::remove_file(&old_socket);
        if let Some(p) = old_shmem_path {
            let _ = std::fs::remove_file(&p);
        }

        let (shmem, shmem_path) =
            SharedTripleBuffer::new_persistent(width, height, 4, None)
                .map_err(|e| IpcError::SpawnFailed(format!("shmem realloc failed: {}", e)))?;

        let _old = self.tabs.remove(&tab_id);
        match self
            .spawn_and_accept(tab_id, generation, &socket_path, shmem, shmem_path.clone(), &initial_url)
            .await
        {
            Ok(()) => Ok(()),
            Err(e) => {
                let status = Arc::new(StdMutex::new(TabStatus::Crashed {
                    reason: format!("reload failed: {}", e),
                }));
                self.tabs.insert(
                    tab_id,
                    TabProcessHandle {
                        tab_id,
                        status,
                        writer: None,
                        heartbeat: Arc::new(StdMutex::new(Instant::now())),
                        shmem: None,
                        shmem_path: None,
                        socket_path,
                        child: None,
                        initial_url,
                        inbox: Arc::new(StdMutex::new(Vec::new())),
                    },
                );
                Err(e)
            }
        }
    }

    pub async fn close_tab(&mut self, tab_id: u32) {
        if let Some(tab) = self.tabs.remove(&tab_id) {
            if let Ok(mut s) = tab.status.lock() {
                *s = TabStatus::Closed;
            }
            if let Some(child) = tab.child {
                let mut guard = child.lock().await;
                let _ = guard.kill().await;
                let _ = guard.wait().await;
            }
            let _ = std::fs::remove_file(&tab.socket_path);
            if let Some(p) = tab.shmem_path {
                let _ = std::fs::remove_file(&p);
            }
            info!("Closed tab #{}", tab_id);
        }
    }

    pub async fn kill_child_for_test(&mut self, tab_id: u32) -> Result<(), IpcError> {
        let tab = self.tabs.get(&tab_id).ok_or(IpcError::NoSuchTab(tab_id))?;
        if let Some(child) = &tab.child {
            let mut guard = child.lock().await;
            guard.kill().await.map_err(IpcError::Io)?;
            Ok(())
        } else {
            Err(IpcError::ChildExited("no child process".to_string()))
        }
    }

    pub fn poll_child_exits(&mut self) {
        let ids: Vec<u32> = self.tabs.keys().copied().collect();
        for id in ids {
            let dead = self
                .tabs
                .get(&id)
                .and_then(|t| t.child.clone())
                .and_then(|child| {
                    child.try_lock().ok().and_then(|mut g| {
                        g.try_wait().ok().and_then(|opt| opt.map(|st| !st.success()))
                    })
                })
                .unwrap_or(false);
            if dead {
                self.mark_tab_crashed(id, "Tab process exited (poll)".to_string());
            }
        }
    }

    pub async fn send_to_tab(&self, tab_id: u32, msg: &IpcMessage) -> Result<(), IpcError> {
        if let Some(tab) = self.tabs.get(&tab_id) {
            if let Some(writer) = &tab.writer {
                let mut guard = writer.lock().await;
                guard.write_message(msg).await?;
                Ok(())
            } else {
                Err(IpcError::ChildExited(format!(
                    "tab {} has no live IPC channel",
                    tab_id
                )))
            }
        } else {
            Err(IpcError::NoSuchTab(tab_id))
        }
    }

    pub fn check_watchdog(&mut self, timeout: Duration) -> Vec<u32> {
        let now = Instant::now();
        let mut timed_out = Vec::new();

        for (id, tab) in self.tabs.iter_mut() {
            let watchable = tab
                .status
                .lock()
                .map(|s| matches!(*s, TabStatus::Running { .. } | TabStatus::Initializing))
                .unwrap_or(false);
            if !watchable {
                continue;
            }
            let stale = tab
                .heartbeat
                .lock()
                .map(|hb| now.duration_since(*hb) > timeout)
                .unwrap_or(false);
            if stale {
                if let Ok(mut s) = tab.status.lock() {
                    *s = TabStatus::Crashed {
                        reason: "Tab Process Watchdog Timeout (Unresponsive)".to_string(),
                    };
                }
                timed_out.push(*id);
            }
        }

        timed_out
    }
}

pub struct TabWorker<R, W> {
    pub tab_id: u32,
    reader: IpcReader<R>,
    writer: IpcWriter<W>,
    is_running: Arc<AtomicBool>,
    shmem: Option<SharedTripleBuffer>,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> TabWorker<R, W> {
    pub fn with_halves(
        tab_id: u32,
        reader: R,
        writer: W,
        shmem: Option<SharedTripleBuffer>,
    ) -> Self {
        Self {
            tab_id,
            reader: IpcReader::new(reader),
            writer: IpcWriter::new(writer),
            is_running: Arc::new(AtomicBool::new(true)),
            shmem,
        }
    }

    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
    }

    pub async fn run_loop(&mut self) -> Result<(), IpcError> {
        while self.is_running.load(Ordering::Relaxed) {
            let msg = match self.reader.read_message().await {
                Ok(m) => m,
                Err(IpcError::ConnectionClosed) => break,
                Err(e) => return Err(e),
            };
            match msg {
                IpcMessage::NavigateTo { tab_id, url } => {
                    let title = format!("Title for {}", url);
                    self.writer
                        .write_message(&IpcMessage::PageLoaded {
                            tab_id,
                            url: url.clone(),
                            title,
                        })
                        .await?;
                    if self.shmem.is_some() {
                        let (width, height, stride) = {
                            let sh = self.shmem.as_ref().expect("checked above");
                            let (w, h, s, _) = sh.dimensions();
                            (w, h, s)
                        };
                        for frame_idx in 1u8..=5u8 {
                            let idx = {
                                let sh = self.shmem.as_ref().expect("checked above");
                                sh.next_writable_buffer_index()
                            };
                            {
                                let sh = self.shmem.as_mut().expect("checked above");
                                let slice =
                                    sh.get_buffer_mut(idx as usize).map_err(|e| {
                                        IpcError::SpawnFailed(format!("shmem write failed: {}", e))
                                    })?;
                                slice[0] = frame_idx;
                                slice[slice.len() - 1] = frame_idx.wrapping_mul(10);
                            }
                            let frame_number = {
                                let sh = self.shmem.as_ref().expect("checked above");
                                let n = sh.commit_frame(idx);
                                let _ = sh.flush();
                                n
                            };
                            self.writer
                                .write_message(&IpcMessage::RenderFrameReady {
                                    tab_id,
                                    buffer_index: idx,
                                    frame_number,
                                    width,
                                    height,
                                    stride,
                                })
                                .await?;
                        }
                    }
                }
                IpcMessage::HeartbeatPing { sequence } => {
                    self.writer
                        .write_message(&IpcMessage::HeartbeatPong { sequence })
                        .await?;
                }
                IpcMessage::TabClosed { tab_id } if tab_id == self.tab_id => break,
                IpcMessage::TabCreated { .. }
                | IpcMessage::TabClosed { .. }
                | IpcMessage::PageLoaded { .. }
                | IpcMessage::RenderFrameReady { .. }
                | IpcMessage::CrashNotification { .. }
                | IpcMessage::HeartbeatPong { .. }
                | IpcMessage::InputEvent { .. }
                | IpcMessage::Resize { .. } => {}
            }
        }
        Ok(())
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> TabWorker<tokio::io::ReadHalf<S>, tokio::io::WriteHalf<S>> {
    pub fn from_stream(
        tab_id: u32,
        stream: S,
        shmem: Option<SharedTripleBuffer>,
    ) -> Self {
        let (r, w) = tokio::io::split(stream);
        Self::with_halves(tab_id, r, w, shmem)
    }
}

pub async fn run_tab_worker_process(
    socket_path: &Path,
    tab_id: u32,
    shmem_path: &Path,
) -> Result<(), IpcError> {
    let shmem = SharedTripleBuffer::open_shared(shmem_path)
        .map_err(|e| IpcError::SpawnFailed(format!("open shmem failed: {}", e)))?;

    #[cfg(unix)]
    {
        let mut last_err = String::new();
        for _ in 0..100 {
            match tokio::net::UnixStream::connect(socket_path).await {
                Ok(stream) => {
                    let mut worker = TabWorker::from_stream(tab_id, stream, Some(shmem));
                    return worker.run_loop().await;
                }
                Err(e) => {
                    last_err = e.to_string();
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
        return Err(IpcError::SpawnFailed(format!(
            "connect to {} failed: {}",
            socket_path.display(),
            last_err
        )));
    }

    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        let mut last_err = String::new();
        for _ in 0..100 {
            match ClientOptions::new().open(socket_path) {
                Ok(stream) => {
                    let mut worker = TabWorker::from_stream(tab_id, stream, Some(shmem));
                    return worker.run_loop().await;
                }
                Err(e) => {
                    last_err = e.to_string();
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
        return Err(IpcError::SpawnFailed(format!(
            "connect to pipe failed: {}",
            last_err
        )));
    }
}
