use crate::page_layout_host::{LayoutInputs, PageLayoutHost};
use crate::page_style::{document_base_url, resolve_external_resource};
use crate::text_editing::{edit_text, KeyboardInput, TextSelection};
use boa_engine::builtins::promise::PromiseState;
use boa_engine::module::{Module, ModuleLoader, ModuleRequest, Referrer};
use rix_dom::{CanvasBitmap, Document, NodeData, NodeId, VideoFrameBitmap};
use rix_html::{HtmlParserStream, ParserStreamStep};
use rix_js::{
    dom_bridge::{
        dispatch_dom_event_to_js, register_dom_bridge, register_layout_api, DomBridgeState,
    },
    runtime::{JsRuntime, JsRuntimeConfig},
    web_api::{register_web_apis, EventLoop, WebApiState},
};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    rc::Rc,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PAGE_CONSOLE_MESSAGES: usize = 512;
const MAX_PAGE_CONSOLE_MESSAGE_CHARS: usize = 4096;
const MAX_RETAINED_CONSOLE_MESSAGES: usize = MAX_PAGE_CONSOLE_MESSAGES + 1;
const MAX_RETAINED_SCRIPT_TIMINGS: usize = 512;
const MAX_PAGE_PROBE_SCRIPT_BYTES: u64 = 64 * 1024;
const WORKER_WATCHDOG: Duration = Duration::from_secs(30);
const WORKER_STARTUP_WATCHDOG: Duration = Duration::from_secs(60);
const MODULE_LOAD_TIMEOUT: Duration = Duration::from_secs(15);

type ModuleLoadWaiters = Arc<Mutex<HashMap<u64, mpsc::Sender<ModuleLoadResponse>>>>;

struct ModuleLoadResponse {
    source_url: String,
    result: Result<String, String>,
}

struct BrowserModuleLoader {
    base_url: String,
    next_request_id: AtomicU64,
    request_count: AtomicUsize,
    request_limit: usize,
    waiters: ModuleLoadWaiters,
    modules: RefCell<HashMap<String, Module>>,
}

impl BrowserModuleLoader {
    fn new(base_url: String, request_limit: usize, waiters: ModuleLoadWaiters) -> Self {
        Self {
            base_url,
            next_request_id: AtomicU64::new(1),
            request_count: AtomicUsize::new(0),
            request_limit,
            waiters,
            modules: RefCell::new(HashMap::new()),
        }
    }

    fn register_root(&self, source_url: &str, module: Module) {
        self.modules
            .borrow_mut()
            .entry(source_url.to_string())
            .or_insert(module);
    }
}

impl ModuleLoader for BrowserModuleLoader {
    async fn load_imported_module(
        self: Rc<Self>,
        referrer: Referrer,
        request: ModuleRequest,
        context: &RefCell<&mut boa_engine::Context>,
    ) -> boa_engine::JsResult<Module> {
        if !request.attributes().is_empty() {
            return Err(boa_engine::JsNativeError::typ()
                .with_message("module import attributes are not supported")
                .into());
        }
        let request_index = self.request_count.fetch_add(1, Ordering::Relaxed);
        if request_index >= self.request_limit {
            return Err(boa_engine::JsNativeError::error()
                .with_message("module request limit exceeded")
                .into());
        }

        let specifier = request.specifier().to_std_string_escaped();
        if !is_url_like_module_specifier(&specifier) {
            return Err(boa_engine::JsNativeError::typ()
                .with_message(format!("unmapped bare module specifier: {specifier}"))
                .into());
        }
        let referrer_url = referrer
            .path()
            .map(|path| path.to_string_lossy().into_owned())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| self.base_url.clone());
        let Some((resolved_url, _)) = resolve_external_resource(&referrer_url, &specifier) else {
            return Err(boa_engine::JsNativeError::typ()
                .with_message(format!("cannot resolve module specifier: {specifier}"))
                .into());
        };
        if let Some(module) = self.modules.borrow().get(&resolved_url).cloned() {
            return Ok(module);
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        let registered = self
            .waiters
            .lock()
            .map_err(|_| {
                boa_engine::JsNativeError::error()
                    .with_message("module response router is unavailable")
            })?
            .insert(request_id, sender);
        if registered.is_some() {
            return Err(boa_engine::JsNativeError::error()
                .with_message("duplicate module request identifier")
                .into());
        }
        if let Err(error) = send(Update::ModuleLoadRequest {
            request_id,
            source_url: resolved_url.clone(),
        }) {
            if let Ok(mut waiters) = self.waiters.lock() {
                waiters.remove(&request_id);
            }
            return Err(boa_engine::JsNativeError::error()
                .with_message(error)
                .into());
        }
        let response = receiver
            .recv_timeout(MODULE_LOAD_TIMEOUT)
            .map_err(|error| {
                if let Ok(mut waiters) = self.waiters.lock() {
                    waiters.remove(&request_id);
                }
                boa_engine::JsNativeError::error()
                    .with_message(format!("module fetch did not complete: {error}"))
            })?;
        let source = response.result.map_err(|error| {
            boa_engine::JsNativeError::error()
                .with_message(format!("module fetch failed for {resolved_url}: {error}"))
        })?;
        let source = boa_engine::Source::from_bytes(&source)
            .with_path(std::path::Path::new(&response.source_url));
        let module = Module::parse(source, None, &mut context.borrow_mut())?;
        let mut modules = self.modules.borrow_mut();
        modules.insert(resolved_url, module.clone());
        modules.insert(response.source_url, module.clone());
        Ok(module)
    }

    fn init_import_meta(
        self: Rc<Self>,
        import_meta: &boa_engine::JsObject,
        module: &Module,
        context: &mut boa_engine::Context,
    ) {
        let Some(url) = module
            .path()
            .map(|path| path.to_string_lossy().into_owned())
        else {
            return;
        };
        if let Err(error) = import_meta.set(
            boa_engine::JsString::from("url"),
            boa_engine::JsValue::from(boa_engine::JsString::from(url)),
            false,
            context,
        ) {
            tracing::warn!(%error, "Could not initialize import.meta.url");
        }
    }
}

fn is_url_like_module_specifier(specifier: &str) -> bool {
    specifier.starts_with('/')
        || specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with('?')
        || specifier.starts_with('#')
        || specifier.split_once(':').is_some_and(|(scheme, _)| {
            !scheme.is_empty()
                && scheme.chars().enumerate().all(|(index, character)| {
                    character.is_ascii_alphabetic()
                        || (index > 0
                            && (character.is_ascii_digit() || matches!(character, '+' | '-' | '.')))
                })
        })
}

pub type NodeToken = u64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotNode {
    pub token: NodeToken,
    pub parent: Option<NodeToken>,
    pub data: SnapshotData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotName {
    namespace: String,
    prefix: Option<String>,
    local: String,
}

impl SnapshotName {
    fn from_name(name: &markup5ever::QualName) -> Self {
        Self {
            namespace: name.ns.to_string(),
            prefix: name.prefix.as_ref().map(ToString::to_string),
            local: name.local.to_string(),
        }
    }

    fn into_name(self) -> markup5ever::QualName {
        markup5ever::QualName::new(
            self.prefix.map(Into::into),
            self.namespace.into(),
            self.local.into(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotData {
    Document,
    DocumentFragment,
    ShadowRoot {
        host: NodeToken,
        mode: String,
        delegates_focus: bool,
        #[serde(default)]
        clonable: bool,
    },
    DocumentType {
        name: String,
        public_id: String,
        system_id: String,
    },
    Element {
        name: SnapshotName,
        attributes: Vec<(SnapshotName, String)>,
        template_contents: Option<NodeToken>,
        canvas_bitmap: Option<CanvasBitmap>,
        video_frame: Option<VideoFrameBitmap>,
        shadow_root: Option<NodeToken>,
        content_document: Option<NodeToken>,
    },
    Text(String),
    Comment(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SnapshotState {
    pub nodes: Vec<SnapshotNode>,
    pub focus: FocusState,
    #[serde(default)]
    pub dynamic_stylesheets: HashMap<NodeToken, String>,
    #[serde(default)]
    pub adopted_stylesheets: HashMap<NodeToken, Vec<String>>,
}

impl SnapshotState {
    pub fn document(&self) -> Result<(Document, HashMap<NodeToken, NodeId>), String> {
        let mut document = Document::new();
        let mut nodes = HashMap::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if node.token == 0 || nodes.contains_key(&node.token) {
                return Err("Invalid or duplicate page node token".into());
            }
            if node
                .parent
                .is_some_and(|parent| !nodes.contains_key(&parent))
            {
                return Err("Page snapshot parent must precede its child".into());
            }
            let id = match node.data.clone() {
                SnapshotData::Document if index == 0 && node.parent.is_none() => document.root(),
                SnapshotData::Document if node.parent.is_none() => {
                    document.new_node(NodeData::Document)
                }
                SnapshotData::Document => return Err("Invalid page document root".into()),
                SnapshotData::DocumentFragment => document.new_node(NodeData::DocumentFragment),
                SnapshotData::ShadowRoot {
                    host,
                    mode,
                    delegates_focus,
                    clonable,
                } => {
                    let Some(host) = nodes.get(&host).copied() else {
                        return Err("Missing shadow host node".into());
                    };
                    document.new_node(NodeData::ShadowRoot {
                        host,
                        mode,
                        delegates_focus,
                        clonable,
                    })
                }
                SnapshotData::DocumentType {
                    name,
                    public_id,
                    system_id,
                } => document.new_node(NodeData::DocumentType {
                    name,
                    public_id,
                    system_id,
                }),
                SnapshotData::Element {
                    name,
                    attributes,
                    canvas_bitmap,
                    video_frame,
                    ..
                } => {
                    let mut element = rix_dom::ElementData::new(name.into_name());
                    element.attributes = attributes
                        .into_iter()
                        .map(|(name, value)| (name.into_name(), value))
                        .collect();
                    element.canvas_bitmap = canvas_bitmap;
                    element.video_frame = video_frame;
                    document.new_node(NodeData::Element(element))
                }
                SnapshotData::Text(text) => document.create_text(&text),
                SnapshotData::Comment(comment) => document.create_comment(&comment),
            };
            if index == 0 && !matches!(node.data, SnapshotData::Document) {
                return Err("Page snapshot requires a document root".into());
            }
            if let Some(parent) = node.parent {
                document.append_child(nodes[&parent], id);
            }
            nodes.insert(node.token, id);
        }
        if self.nodes.is_empty() {
            return Err("Empty page snapshot".into());
        }
        for node in &self.nodes {
            if let SnapshotData::Element {
                template_contents: Some(fragment),
                ..
            } = node.data
            {
                let fragment = nodes
                    .get(&fragment)
                    .copied()
                    .ok_or("Missing template content node")?;
                if !matches!(document.get(fragment), Some(NodeData::DocumentFragment)) {
                    return Err("Invalid template content node".into());
                }
                if let Some(NodeData::Element(element)) = document.get_mut(nodes[&node.token]) {
                    element.template_contents = Some(fragment);
                }
            }
            if let SnapshotData::Element {
                shadow_root: Some(root),
                ..
            } = node.data
            {
                let root = nodes
                    .get(&root)
                    .copied()
                    .ok_or("Missing element shadow root")?;
                let Some(NodeData::ShadowRoot { host, .. }) = document.get(root) else {
                    return Err("Invalid element shadow root".into());
                };
                if *host != nodes[&node.token] {
                    return Err("Shadow root host does not match its element".into());
                }
                if let Some(NodeData::Element(element)) = document.get_mut(nodes[&node.token]) {
                    element.shadow_root = Some(root);
                }
            }
            if let SnapshotData::Element {
                content_document: Some(doc_root),
                ..
            } = node.data
            {
                let doc_root = nodes
                    .get(&doc_root)
                    .copied()
                    .ok_or("Missing element content document")?;
                if !matches!(document.get(doc_root), Some(NodeData::Document)) {
                    return Err("Invalid content document node".into());
                }
                if let Some(NodeData::Element(element)) = document.get_mut(nodes[&node.token]) {
                    element.content_document = Some(doc_root);
                }
            }
        }
        let focused = self
            .focus
            .target
            .and_then(|token| nodes.get(&token).copied());
        document.set_focused(focused, true);
        document.set_text_selection(focused.and_then(|node| {
            self.focus
                .selection
                .map(|selection| (node, selection.anchor, selection.cursor))
        }));
        for (token, css) in &self.dynamic_stylesheets {
            if let Some(id) = nodes.get(token) {
                document.set_dynamic_stylesheet(*id, css.clone());
            }
        }
        for (token, sheets) in &self.adopted_stylesheets {
            if let Some(id) = nodes.get(token) {
                document.set_adopted_stylesheets(*id, sheets.clone());
            }
        }
        Ok((document, nodes))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusState {
    pub epoch: u64,
    pub revision: u64,
    pub target: Option<NodeToken>,
    pub selection: Option<TextSelection>,
}

#[derive(Clone)]
pub struct WorkerCommand {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
}

#[derive(Serialize, Deserialize)]
enum Request {
    Focus {
        target: Option<NodeToken>,
        epoch: u64,
        selection: TextSelection,
    },
    Bootstrap {
        url: String,
        html: String,
        scripts: Vec<PageScript>,
        #[serde(default = "default_page_visibility")]
        page_visible: bool,
        #[serde(default = "default_dynamic_script_request_limit")]
        dynamic_script_request_limit: usize,
        #[serde(default)]
        navigation: Option<NavigationPerformance>,
        #[serde(default)]
        layout: LayoutInputs,
        #[serde(default)]
        cookies: Vec<rix_storage::models::Cookie>,
        #[serde(default)]
        local_storage: HashMap<String, String>,
        #[serde(default)]
        theme: Option<String>,
    },
    Layout(LayoutChange),
    Event {
        id: u64,
        target: NodeToken,
        event: String,
        keyboard: Option<KeyboardInput>,
        value: Option<String>,
    },
    ScriptLoadResult {
        target: NodeToken,
        source_url: String,
        result: Result<String, String>,
    },
    ModuleLoadResult {
        request_id: u64,
        source_url: String,
        result: Result<String, String>,
    },
    SetPageVisibility {
        visible: bool,
    },
    SetTheme {
        theme: String,
    },
    AnimationFrameOpportunity {
        timestamp_ms: f64,
    },
    TraverseHistory {
        amount: i32,
    },
    MediaSamples {
        session_id: rix_ipc::media::MediaSessionId,
        track_id: u32,
        samples: Vec<rix_media::MediaSample>,
        codec: String,
        extra_data: Vec<u8>,
        sample_rate: u32,
        channels: u16,
    },
    MediaEngineOpened {
        session_id: rix_ipc::media::MediaSessionId,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum LayoutChange {
    Viewport {
        width: f32,
        height: f32,
    },
    Scroll {
        x: f32,
        y: f32,
    },
    ImageSize {
        url: String,
        width: f32,
        height: f32,
    },
    Stylesheet {
        url: String,
        css: String,
    },
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct NavigationPerformance {
    pub timing: rix_net::NavigationTiming,
    pub elapsed_ms: f64,
}

#[derive(Serialize, Deserialize)]
pub enum Update {
    Busy,
    Idle,
    Snapshot { html: String, state: SnapshotState },
    FocusChanged { focus: FocusState },
    ScrollRequested { x: f32, y: f32 },
    Error { message: String },
    Console { level: String, message: String },
    ResourceLoad {
        record: rix_js::web_api::ResourceLoadRecord,
    },
    ResourceLoadTraceTruncated,
    PendingNetworkRequests { count: usize },
    AnimationFrameRequested,
    AnimationFrameComplete,
    DynamicScriptRequest {
        target: NodeToken,
        source_url: String,
    },
    ModuleLoadRequest { request_id: u64, source_url: String },
    CookieSet {
        cookie: rix_storage::models::Cookie,
    },
    StorageMutated {
        mutation: rix_js::web_api::StorageMutation,
    },
    ScriptStarted {
        index: usize,
        source_bytes: usize,
        source_url: String,
    },
    ScriptPhase {
        index: usize,
        phase: String,
    },
    TaskStarted { label: String },
    ScriptTiming { timing: PageScriptTiming },
    Ready,
    EventHandled {
        id: u64,
        allowed: bool,
        target: Option<NodeToken>,
        focus_epoch: u64,
    },
    TextEdited {
        id: u64,
        target: NodeToken,
        focus_epoch: u64,
        value: String,
        selection: TextSelection,
    },
    NavigationRequest {
        url: String,
        kind: rix_js::web_api::PageNavigationKind,
    },
    HistoryChanged {
        url: String,
        kind: rix_js::web_api::HistoryChangeKind,
        index: u32,
    },
    HistoryTraversalUnavailable {
        amount: i32,
    },
    MediaSamples {
        session_id: rix_ipc::media::MediaSessionId,
        track_id: u32,
        samples: Vec<rix_media::MediaSample>,
        codec: String,
        extra_data: Vec<u8>,
        sample_rate: u32,
        channels: u16,
    },
    MediaEngineOpened {
        session_id: rix_ipc::media::MediaSessionId,
    },
    MediaPlayback {
        session_id: rix_ipc::media::MediaSessionId,
        command: rix_ipc::media::MediaWorkerCommand,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageConsoleMessage {
    pub level: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageScriptTiming {
    pub index: usize,
    pub source_bytes: usize,
    #[serde(default)]
    pub source_url: String,
    pub evaluation_ms: f64,
    pub parse_ms: f64,
    pub compile_ms: f64,
    pub execute_ms: f64,
    pub promise_jobs_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PageScriptInProgress {
    pub index: usize,
    pub source_bytes: usize,
    pub source_url: String,
}

fn watchdog_failure_message(
    active_script: Option<&PageScriptInProgress>,
    active_task: Option<&str>,
) -> String {
    if let Some(script) = active_script {
        format!(
            "Page runtime stopped: script {} ({} bytes, {}) did not yield within watchdog deadline",
            script.index,
            script.source_bytes,
            script.source_url.chars().take(256).collect::<String>()
        )
    } else if let Some(task) = active_task {
        format!("Page runtime stopped during {task}; task did not yield within watchdog deadline")
    } else {
        "Page runtime stopped: task did not yield within watchdog deadline".into()
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PageScriptKind {
    #[default]
    InlineClassic,
    ExternalClassic,
    DynamicClassic,
    InlineModule,
    ExternalModule,
    DiagnosticProbe,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageScript {
    pub source: String,
    pub element_index: usize,
    #[serde(default)]
    pub kind: PageScriptKind,
    #[serde(default)]
    pub source_url: String,
}

pub(crate) use rix_js::dom_bridge::script::{is_classic_script_type, is_module_script_type};

pub struct PageWorker {
    child: Child,
    requests: SyncSender<Request>,
    updates: Receiver<Update>,
    busy_since: Option<Instant>,
    stopped: bool,
    next_event: Cell<u64>,
    focus_epoch: Cell<u64>,
    latest_revision: Cell<u64>,
    pending_focus: RefCell<Option<Request>>,
    pending_script_results: RefCell<VecDeque<Request>>,
    pending_module_results: RefCell<VecDeque<Request>>,
    pending_page_visibility: Cell<Option<bool>>,
    performance_started_at: Instant,
    performance_elapsed_at_start_ms: f64,
    pending_layout: RefCell<std::collections::VecDeque<LayoutChange>>,
    pending_history_traversals: RefCell<std::collections::VecDeque<i32>>,
    installed_nodes: HashMap<NodeId, NodeToken>,
    installed_ids: HashMap<NodeToken, NodeId>,
    installed_snapshot: Vec<SnapshotNode>,
    installed_focus: Option<NodeToken>,
    console_messages: Vec<PageConsoleMessage>,
    script_timings: Vec<PageScriptTiming>,
    active_script: Option<PageScriptInProgress>,
    active_task: Option<String>,
    pending_network_requests: usize,
    pub ready: bool,
}

impl PageWorker {
    pub fn spawn(
        command: &WorkerCommand,
        url: &str,
        html: &str,
        scripts: Vec<String>,
    ) -> Result<Self, String> {
        Self::spawn_with_navigation(command, url, html, scripts, None, LayoutInputs::default())
    }

    pub fn spawn_with_navigation(
        command: &WorkerCommand,
        url: &str,
        html: &str,
        scripts: Vec<String>,
        navigation: Option<NavigationPerformance>,
        layout: LayoutInputs,
    ) -> Result<Self, String> {
        let scripts = scripts
            .into_iter()
            .enumerate()
            .map(|(element_index, source)| PageScript {
                source,
                element_index,
                kind: PageScriptKind::InlineClassic,
                source_url: url.to_string(),
            })
            .collect();
        Self::spawn_with_navigation_scripts(command, url, html, scripts, navigation, layout)
    }

    pub fn spawn_with_navigation_scripts(
        command: &WorkerCommand,
        url: &str,
        html: &str,
        scripts: Vec<PageScript>,
        navigation: Option<NavigationPerformance>,
        layout: LayoutInputs,
    ) -> Result<Self, String> {
        Self::spawn_with_dynamic_script_limit(
            command,
            url,
            html,
            scripts,
            navigation,
            layout,
            default_dynamic_script_request_limit(),
        )
    }

    pub fn spawn_with_dynamic_script_limit(
        command: &WorkerCommand,
        url: &str,
        html: &str,
        scripts: Vec<PageScript>,
        navigation: Option<NavigationPerformance>,
        layout: LayoutInputs,
        dynamic_script_request_limit: usize,
    ) -> Result<Self, String> {
        Self::spawn_with_visibility(
            command,
            url,
            html,
            scripts,
            navigation,
            layout,
            dynamic_script_request_limit,
            true,
        )
    }

    pub fn spawn_with_visibility(
        command: &WorkerCommand,
        url: &str,
        html: &str,
        scripts: Vec<PageScript>,
        navigation: Option<NavigationPerformance>,
        layout: LayoutInputs,
        dynamic_script_request_limit: usize,
        page_visible: bool,
    ) -> Result<Self, String> {
        Self::spawn_with_visibility_and_cookies(
            command,
            url,
            html,
            scripts,
            navigation,
            layout,
            dynamic_script_request_limit,
            page_visible,
            Vec::new(),
        )
    }

    pub fn spawn_with_visibility_and_cookies(
        command: &WorkerCommand,
        url: &str,
        html: &str,
        scripts: Vec<PageScript>,
        navigation: Option<NavigationPerformance>,
        layout: LayoutInputs,
        dynamic_script_request_limit: usize,
        page_visible: bool,
        cookies: Vec<rix_storage::models::Cookie>,
    ) -> Result<Self, String> {
        Self::spawn_with_storage(
            command,
            url,
            html,
            None,
            scripts,
            navigation,
            layout,
            dynamic_script_request_limit,
            page_visible,
            cookies,
            HashMap::new(),
            None,
        )
    }

    pub fn spawn_with_storage(
        command: &WorkerCommand,
        url: &str,
        html: &str,
        document: Option<&Document>,
        scripts: Vec<PageScript>,
        navigation: Option<NavigationPerformance>,
        layout: LayoutInputs,
        dynamic_script_request_limit: usize,
        page_visible: bool,
        cookies: Vec<rix_storage::models::Cookie>,
        local_storage: HashMap<String, String>,
        theme: Option<String>,
    ) -> Result<Self, String> {
        let performance_started_at = Instant::now();
        let performance_elapsed_at_start_ms =
            navigation.map_or(0.0, |performance| performance.elapsed_ms);
        let worker_stderr = if std::env::var_os("RIX_WORKER_TRACE").is_some() {
            Stdio::inherit()
        } else {
            Stdio::null()
        };
        let mut child = Command::new(&command.executable)
            .args(&command.arguments)
            .arg("--page-runtime-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(worker_stderr)
            .spawn()
            .map_err(|error| error.to_string())?;
        let mut input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, requests) = mpsc::sync_channel::<Request>(64);
        let (updates, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(request) = requests.recv() {
                let Ok(encoded) = serde_json::to_vec(&request) else {
                    break;
                };
                if encoded.len() > MAX_MESSAGE_BYTES
                    || input
                        .write_all(&encoded)
                        .and_then(|_| input.write_all(b"\n"))
                        .is_err()
                {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            while let Ok(Some(line)) = read_message(&mut reader) {
                let Ok(update) = serde_json::from_slice(&line) else {
                    break;
                };
                if updates.send(update).is_err() {
                    break;
                }
            }
        });
        sender
            .send(Request::Bootstrap {
                url: url.to_string(),
                html: html.to_string(),
                scripts,
                page_visible,
                dynamic_script_request_limit,
                navigation,
                layout,
                cookies,
                local_storage,
                theme,
            })
            .map_err(|error| error.to_string())?;
        let (tokens, nodes) = if let Some(doc) = document {
            let mut tokens = NodeTokens::default();
            let nodes = tokens.snapshot_nodes(doc);
            (tokens, nodes)
        } else {
            let initial = rix_html::parse_html(html);
            let mut tokens = NodeTokens::default();
            let nodes = tokens.snapshot_nodes(&initial);
            (tokens, nodes)
        };
        Ok(Self {
            child,
            requests: sender,
            updates: receiver,
            busy_since: Some(Instant::now()),
            stopped: false,
            next_event: Cell::new(1),
            focus_epoch: Cell::new(0),
            latest_revision: Cell::new(0),
            pending_focus: RefCell::new(None),
            pending_script_results: RefCell::new(VecDeque::new()),
            pending_module_results: RefCell::new(VecDeque::new()),
            pending_page_visibility: Cell::new(None),
            performance_started_at,
            performance_elapsed_at_start_ms,
            pending_layout: RefCell::new(Default::default()),
            pending_history_traversals: RefCell::new(Default::default()),
            installed_nodes: tokens.by_node,
            installed_ids: tokens.by_token,
            installed_snapshot: nodes,
            installed_focus: None,
            console_messages: Vec::new(),
            script_timings: Vec::new(),
            active_script: None,
            active_task: None,
            pending_network_requests: 0,
            ready: false,
        })
    }

    pub fn event(
        &self,
        doc: &Document,
        node: NodeId,
        event: &str,
        key: Option<String>,
        value: Option<String>,
    ) -> Option<u64> {
        if self.stopped || !self.flush_focus() {
            return None;
        }
        let target = self.node_token(doc, node)?;
        let id = self.next_event.get();
        self.next_event.set(id.wrapping_add(1));
        self.requests
            .try_send(Request::Event {
                id,
                target,
                event: event.to_string(),
                keyboard: key.map(|key| KeyboardInput {
                    key,
                    ..Default::default()
                }),
                value,
            })
            .ok()?;
        Some(id)
    }

    pub fn complete_script_load(
        &self,
        target: NodeToken,
        source_url: String,
        result: Result<String, String>,
    ) -> bool {
        const MAX_PENDING_SCRIPT_RESULTS: usize = 64;
        if self.stopped {
            return false;
        }
        let request = Request::ScriptLoadResult {
            target,
            source_url,
            result,
        };
        match self.requests.try_send(request) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(request)) => {
                let mut pending = self.pending_script_results.borrow_mut();
                if pending.len() >= MAX_PENDING_SCRIPT_RESULTS {
                    return false;
                }
                pending.push_back(request);
                true
            }
            Err(mpsc::TrySendError::Disconnected(_)) => false,
        }
    }

    pub fn complete_module_load(
        &self,
        request_id: u64,
        source_url: String,
        result: Result<String, String>,
    ) -> bool {
        const MAX_PENDING_MODULE_RESULTS: usize = 128;
        if self.stopped {
            return false;
        }
        let request = Request::ModuleLoadResult {
            request_id,
            source_url,
            result,
        };
        match self.requests.try_send(request) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(request)) => {
                let mut pending = self.pending_module_results.borrow_mut();
                if pending.len() >= MAX_PENDING_MODULE_RESULTS {
                    return false;
                }
                pending.push_back(request);
                true
            }
            Err(mpsc::TrySendError::Disconnected(_)) => false,
        }
    }

    pub fn set_page_visibility(&self, visible: bool) {
        if self.stopped {
            return;
        }
        match self
            .requests
            .try_send(Request::SetPageVisibility { visible })
        {
            Ok(()) => self.pending_page_visibility.set(None),
            Err(mpsc::TrySendError::Full(_)) => {
                self.pending_page_visibility.set(Some(visible));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                tracing::warn!("Page worker disconnected before visibility update");
            }
        }
    }

    pub fn set_theme(&self, theme: &str) -> bool {
        if self.stopped {
            return false;
        }
        self.requests
            .try_send(Request::SetTheme {
                theme: theme.to_string(),
            })
            .is_ok()
    }

    pub fn animation_frame_opportunity(&self, at: Instant) -> bool {
        if self.stopped {
            return false;
        }
        let timestamp_ms = self.performance_elapsed_at_start_ms
            + at.saturating_duration_since(self.performance_started_at)
                .as_secs_f64()
                * 1000.0;
        self.requests
            .try_send(Request::AnimationFrameOpportunity { timestamp_ms })
            .is_ok()
    }

    pub fn key_event(&self, doc: &Document, node: NodeId, keyboard: KeyboardInput) -> Option<u64> {
        if self.stopped || !self.flush_focus() {
            return None;
        }
        let target = self.node_token(doc, node)?;
        let id = self.next_event.get();
        self.next_event.set(id.wrapping_add(1));
        self.requests
            .try_send(Request::Event {
                id,
                target,
                event: "keydown".into(),
                keyboard: Some(keyboard),
                value: None,
            })
            .ok()?;
        Some(id)
    }

    pub fn focus(&self, doc: &Document, node: Option<NodeId>, selection: TextSelection) {
        if self.stopped {
            return;
        }
        let target = match node {
            Some(node) => match self.node_token(doc, node) {
                Some(token) => Some(token),
                None => return,
            },
            None => None,
        };
        let epoch = self.focus_epoch.get().wrapping_add(1);
        self.focus_epoch.set(epoch);
        *self.pending_focus.borrow_mut() = Some(Request::Focus {
            target,
            epoch,
            selection,
        });
        self.flush_focus();
    }

    pub fn layout_change(&self, change: LayoutChange) {
        if self.stopped {
            return;
        }
        {
            let mut pending = self.pending_layout.borrow_mut();
            let replaces = |queued: &LayoutChange| {
                matches!(
                    (queued, &change),
                    (LayoutChange::Viewport { .. }, LayoutChange::Viewport { .. })
                        | (LayoutChange::Scroll { .. }, LayoutChange::Scroll { .. })
                )
            };
            match pending.back_mut() {
                Some(last) if replaces(last) => *last = change,
                _ => pending.push_back(change),
            }
        }
        self.flush_layout();
    }

    fn flush_layout(&self) {
        let mut pending = self.pending_layout.borrow_mut();
        while let Some(change) = pending.pop_front() {
            match self.requests.try_send(Request::Layout(change)) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(Request::Layout(change))) => {
                    pending.push_front(change);
                    return;
                }
                Err(_) => return,
            }
        }
    }

    pub fn traverse_history(&self, amount: i32) {
        if self.stopped {
            return;
        }
        {
            let mut pending = self.pending_history_traversals.borrow_mut();
            pending.push_back(amount);
        }
        self.flush_history_traversals();
    }

    fn flush_history_traversals(&self) {
        let mut pending = self.pending_history_traversals.borrow_mut();
        while let Some(amount) = pending.front().copied() {
            match self
                .requests
                .try_send(Request::TraverseHistory { amount })
            {
                Ok(()) => {
                    pending.pop_front();
                }
                Err(mpsc::TrySendError::Full(_)) => return,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    pending.clear();
                    return;
                }
            }
        }
    }

    fn flush_script_results(&self) {
        let mut pending = self.pending_script_results.borrow_mut();
        while let Some(request) = pending.pop_front() {
            match self.requests.try_send(request) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(request)) => {
                    pending.push_front(request);
                    return;
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    pending.clear();
                    return;
                }
            }
        }
    }

    fn flush_module_results(&self) {
        let mut pending = self.pending_module_results.borrow_mut();
        while let Some(request) = pending.pop_front() {
            match self.requests.try_send(request) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(request)) => {
                    pending.push_front(request);
                    return;
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    pending.clear();
                    return;
                }
            }
        }
    }

    fn flush_page_visibility(&self) {
        let Some(visible) = self.pending_page_visibility.get() else {
            return;
        };
        match self
            .requests
            .try_send(Request::SetPageVisibility { visible })
        {
            Ok(()) => self.pending_page_visibility.set(None),
            Err(mpsc::TrySendError::Full(_)) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.pending_page_visibility.set(None);
            }
        }
    }

    fn flush_focus(&self) -> bool {
        let Some(request) = self.pending_focus.borrow_mut().take() else {
            return true;
        };
        match self.requests.try_send(request) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(request)) => {
                *self.pending_focus.borrow_mut() = Some(request);
                false
            }
            Err(mpsc::TrySendError::Disconnected(_)) => false,
        }
    }

    pub fn node_token(&self, doc: &Document, node: NodeId) -> Option<NodeToken> {
        doc.get(node)?;
        self.installed_nodes.get(&node).copied()
    }

    pub fn resolve_token(&self, doc: &Document, token: NodeToken) -> Option<NodeId> {
        let node = *self.installed_ids.get(&token)?;
        doc.get(node).map(|_| node)
    }

    pub fn install_snapshot(&mut self, state: &SnapshotState, nodes: HashMap<NodeToken, NodeId>) {
        self.installed_nodes = nodes.iter().map(|(token, node)| (*node, *token)).collect();
        self.installed_ids = nodes;
        self.installed_snapshot = state.nodes.clone();
        self.installed_focus = state.focus.target;
    }

    pub fn snapshot_matches(&self, state: &SnapshotState) -> bool {
        self.installed_snapshot == state.nodes && self.installed_focus == state.focus.target
    }

    pub fn focus_epoch(&self) -> u64 {
        self.focus_epoch.get()
    }

    pub fn observe_revision(&self, revision: u64) {
        self.latest_revision
            .set(self.latest_revision.get().max(revision));
    }

    pub fn latest_revision(&self) -> u64 {
        self.latest_revision.get()
    }

    pub fn take_console_messages(&mut self) -> Vec<PageConsoleMessage> {
        std::mem::take(&mut self.console_messages)
    }

    pub fn take_script_timings(&mut self) -> Vec<PageScriptTiming> {
        std::mem::take(&mut self.script_timings)
    }

    pub fn pending_network_requests(&self) -> usize {
        self.pending_network_requests
    }

    pub fn poll(&mut self) -> Vec<Update> {
        self.flush_layout();
        self.flush_history_traversals();
        self.flush_script_results();
        self.flush_module_results();
        self.flush_page_visibility();
        if self.stopped {
            return Vec::new();
        }
        self.flush_focus();
        let mut updates = Vec::new();
        while let Ok(update) = self.updates.try_recv() {
            match &update {
                Update::Busy => {
                    self.busy_since = Some(Instant::now());
                    self.active_task = None;
                }
                Update::Idle => {
                    self.busy_since = None;
                    self.active_task = None;
                }
                Update::Ready => {
                    self.ready = true;
                }
                Update::PendingNetworkRequests { count } => {
                    self.pending_network_requests = *count;
                }
                Update::ScriptStarted {
                    index,
                    source_bytes,
                    source_url,
                } => {
                    self.busy_since = Some(Instant::now());
                    self.active_script = Some(PageScriptInProgress {
                        index: *index,
                        source_bytes: *source_bytes,
                        source_url: source_url.clone(),
                    });
                }
                Update::ScriptPhase { .. } => {
                    self.busy_since = Some(Instant::now());
                }
                Update::TaskStarted { label } => {
                    self.active_script = None;
                    self.active_task = Some(label.clone());
                    self.busy_since = Some(Instant::now());
                }
                Update::ScriptTiming { timing } => {
                    self.active_script = None;
                    self.active_task = None;
                    if self.script_timings.len() < MAX_RETAINED_SCRIPT_TIMINGS {
                        self.script_timings.push(timing.clone());
                    }
                }
                Update::Console { level, message }
                    if self.console_messages.len() < MAX_RETAINED_CONSOLE_MESSAGES =>
                {
                    self.console_messages.push(PageConsoleMessage {
                        level: level.clone(),
                        message: message.clone(),
                    });
                }
                _ => {}
            }
            updates.push(update);
        }
        let exited = self.child.try_wait().ok().flatten();
        let stalled = self.busy_since.is_some_and(|started| {
            started.elapsed()
                >= if self.ready {
                    WORKER_WATCHDOG
                } else {
                    WORKER_STARTUP_WATCHDOG
                }
        });
        let message = match (stalled, exited) {
            (true, _) => Some(watchdog_failure_message(
                self.active_script.as_ref(),
                self.active_task.as_deref(),
            )),
            (false, Some(status)) => Some(format!("Page runtime exited: {status}")),
            (false, None) => None,
        };
        if let Some(message) = message {
            self.stop();
            updates.push(Update::Error { message });
        }
        updates
    }

    pub fn is_running(&self) -> bool {
        !self.stopped
    }

    pub fn is_busy(&self) -> bool {
        self.busy_since.is_some()
    }

    fn stop(&mut self) {
        if !self.stopped {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.stopped = true;
        }
    }
}
impl Drop for PageWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn read_message(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, std::io::Error> {
    let mut message = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(if message.is_empty() {
                None
            } else {
                Some(message)
            });
        }
        let end = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1);
        let count = end.unwrap_or(buffer.len());
        if message.len() + count > MAX_MESSAGE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "page IPC message budget exceeded",
            ));
        }
        message.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(Some(message));
        }
    }
}

pub(crate) fn node_path(doc: &Document, mut node: NodeId) -> Option<Vec<usize>> {
    let mut path = Vec::new();
    while node != doc.root() {
        if let Some(parent) = doc.parent(node) {
            path.push(doc.children(parent).position(|child| child == node)?);
            node = parent;
        } else if let Some(NodeData::ShadowRoot { host, .. }) = doc.get(node) {
            path.push(usize::MAX);
            node = *host;
        } else {
            return None;
        }
    }
    path.reverse();
    Some(path)
}
fn send(update: Update) -> Result<(), String> {
    let update = match update {
        Update::Error { message } => Update::Error {
            message: rix_js::web_api::redact_diagnostic(&message),
        },
        update => update,
    };
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, &update).map_err(|error| error.to_string())?;
    output
        .write_all(b"\n")
        .and_then(|_| output.flush())
        .map_err(|error| error.to_string())
}

fn send_task_started(label: &'static str) -> Result<(), String> {
    if std::env::var_os("RIX_WORKER_TRACE").is_some() {
        send(Update::TaskStarted {
            label: label.to_string(),
        })?;
    }
    Ok(())
}

fn flush_media_playback(runtime: &mut rix_js::runtime::JsRuntime) -> Result<(), String> {
    let store_opt = runtime
        .context_mut()
        .get_data::<Rc<RefCell<rix_js::dom_bridge::media::DemuxerStore>>>()
        .cloned();
    let store = match store_opt {
        Some(store) => store,
        None => return Ok(()),
    };
    let commands = store.borrow_mut().take_pending_playback();
    for queued in commands {
        use rix_js::dom_bridge::media::MediaPlaybackCommand as PageCommand;
        let session_id = rix_ipc::media::MediaSessionId(queued.demuxer_id as u64);
        let command = match queued.command {
            PageCommand::Play { rate } => rix_ipc::media::MediaWorkerCommand::Play {
                session_id,
                playback_rate: rate,
            },
            PageCommand::Pause => rix_ipc::media::MediaWorkerCommand::Pause { session_id },
            PageCommand::Seek { position } => rix_ipc::media::MediaWorkerCommand::Seek {
                session_id,
                target_time: position,
            },
            PageCommand::SetRate { rate } => rix_ipc::media::MediaWorkerCommand::SetPlaybackRate {
                session_id,
                playback_rate: rate,
            },
            PageCommand::SetVolume { volume, muted } => {
                rix_ipc::media::MediaWorkerCommand::SetVolume {
                    session_id,
                    volume,
                    muted,
                }
            }
            PageCommand::EndOfStream => rix_ipc::media::MediaWorkerCommand::EndOfStream {
                session_id,
                error: None,
            },
        };
        send(Update::MediaPlayback {
            session_id,
            command,
        })?;
    }
    Ok(())
}

fn flush_media_samples(runtime: &mut rix_js::runtime::JsRuntime) -> Result<(), String> {
    let store_opt = runtime.context_mut().get_data::<Rc<RefCell<rix_js::dom_bridge::media::DemuxerStore>>>().cloned();
    let store = match store_opt {
        Some(s) => s,
        None => return Ok(()),
    };
    let batches = store.borrow_mut().take_pending_samples();
    for (demuxer_id, samples) in batches {
        use std::collections::BTreeMap;
        let mut by_track: BTreeMap<u32, Vec<rix_media::MediaSample>> = BTreeMap::new();
        for sample in samples {
            by_track.entry(sample.track_id).or_default().push(sample);
        }
        let track_configs: std::collections::BTreeMap<u32, (String, Vec<u8>, u32, u16)> = {
            let store_ref = store.borrow();
            let demuxer_ref = store_ref.demuxer(demuxer_id);
            let mut m = std::collections::BTreeMap::new();
            if let Some(dem) = demuxer_ref {
                for t in dem.tracks() {
                    let cfg = if let Some(ac) = &t.audio_config {
                        (ac.codec.clone(), ac.extra_data.clone(), ac.sample_rate, ac.channels)
                    } else if let Some(vc) = &t.video_config {
                        (vc.codec.clone(), vc.extra_data.clone(), 0u32, 0u16)
                    } else {
                        (String::new(), Vec::new(), 0u32, 0u16)
                    };
                    m.insert(t.id, cfg);
                }
            }
            m
        };
        for (track_id, samples) in by_track {
            let (codec, extra_data, sample_rate, channels) = track_configs
                .get(&track_id)
                .cloned()
                .unwrap_or_else(|| (String::new(), Vec::new(), 0u32, 0u16));
            send(Update::MediaSamples {
                session_id: rix_ipc::media::MediaSessionId(demuxer_id as u64),
                track_id,
                samples,
                codec,
                extra_data,
                sample_rate,
                channels,
            })?;
        }
    }
    Ok(())
}

fn flush_resource_loads(
    state: &Rc<RefCell<rix_js::web_api::WebApiState>>,
    sent: &mut usize,
    truncated_reported: &mut bool,
) -> Result<(), String> {
    const MAX_WORKER_RESOURCE_RECORDS: usize = 512;
    let (records, source_truncated) = {
        let mut state = state.borrow_mut();
        let records = state.take_resource_loads();
        let truncated = state.take_resource_loads_truncated();
        (records, truncated)
    };
    let mut worker_truncated = source_truncated;
    for record in records {
        if *sent < MAX_WORKER_RESOURCE_RECORDS {
            send(Update::ResourceLoad { record })?;
            *sent += 1;
        } else if !*truncated_reported {
            worker_truncated = true;
        }
    }
    if worker_truncated && !*truncated_reported {
        send(Update::ResourceLoadTraceTruncated)?;
        *truncated_reported = true;
    }
    Ok(())
}

fn flush_pending_network_count(
    state: &Rc<RefCell<rix_js::web_api::WebApiState>>,
    last_sent: &mut Option<usize>,
) -> Result<(), String> {
    let count = state.borrow().pending_fetch_count();
    if *last_sent != Some(count) {
        send(Update::PendingNetworkRequests { count })?;
        *last_sent = Some(count);
    }
    Ok(())
}

fn flush_cookie_updates(
    state: &Rc<RefCell<rix_js::web_api::WebApiState>>,
) -> Result<(), String> {
    let updates = state.borrow_mut().take_updated_cookies();
    for cookie in updates {
        send(Update::CookieSet { cookie })?;
    }
    Ok(())
}

fn flush_storage_mutations(
    state: &Rc<RefCell<rix_js::web_api::WebApiState>>,
) -> Result<(), String> {
    let mutations = state.borrow_mut().take_storage_mutations();
    for mutation in mutations {
        send(Update::StorageMutated { mutation })?;
    }
    Ok(())
}

fn flush_navigation_request(
    state: &Rc<RefCell<rix_js::web_api::WebApiState>>,
) -> Result<(), String> {
    let Some(request) = state.borrow_mut().take_navigation_request() else {
        return Ok(());
    };
    send(Update::NavigationRequest {
        url: request.url,
        kind: request.kind,
    })
}

fn flush_history_changes(
    state: &Rc<RefCell<rix_js::web_api::WebApiState>>,
) -> Result<(), String> {
    for change in state.borrow_mut().take_history_changes() {
        send(Update::HistoryChanged {
            url: change.url,
            kind: change.kind,
            index: change.index,
        })?;
    }
    Ok(())
}

fn flush_page_requests(state: &Rc<RefCell<rix_js::web_api::WebApiState>>) -> Result<(), String> {
    flush_navigation_request(state)?;
    flush_history_changes(state)
}

fn flush_scroll_request(host: &PageLayoutHost) -> Result<(), String> {
    if let Some((x, y)) = host.take_scroll_request() {
        send(Update::ScrollRequested { x, y })?;
    }
    Ok(())
}

fn flush_reported_exceptions(
    context: &mut boa_engine::Context,
    source: Option<&PageScript>,
) -> Result<(), String> {
    for message in rix_js::take_reported_exceptions(context) {
        let message = match source {
            Some(script) => script_error_message(script, &message),
            None => message,
        };
        send(Update::Error { message })?;
    }
    Ok(())
}

fn script_error_message(script: &PageScript, message: &str) -> String {
    let message = rix_js::web_api::redact_diagnostic(message);
    let source = if script.source_url.is_empty() {
        "unknown source".to_string()
    } else {
        rix_js::web_api::redact_resource_url(&script.source_url)
    };
    match script.kind {
        PageScriptKind::DiagnosticProbe => format!("page diagnostic probe: {message}"),
        PageScriptKind::DynamicClassic => format!("dynamic script ({source}): {message}"),
        PageScriptKind::InlineModule | PageScriptKind::ExternalModule => {
            format!(
                "module script element {} ({source}): {message}",
                script.element_index + 1
            )
        }
        _ => format!(
            "script element {} ({source}): {message}",
            script.element_index + 1
        ),
    }
}

fn default_dynamic_script_request_limit() -> usize {
    crate::resource_policy::NavigationResourcePolicy::default().max_resources
}

fn read_page_probe_script() -> Result<Option<String>, String> {
    let path = page_probe_script_path_from_args(std::env::args_os().skip(1))?;
    path.map(|path| read_page_probe_script_at_path(&path))
        .transpose()
}

fn page_probe_script_path_from_args(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<Option<PathBuf>, String> {
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == "--rix-page-probe-script" {
            let path = arguments
                .next()
                .ok_or_else(|| "Missing path after --rix-page-probe-script".to_string())?;
            return Ok(Some(PathBuf::from(path)));
        }
    }
    Ok(None)
}

fn read_page_probe_script_at_path(path: &std::path::Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|error| {
        format!(
            "Could not open page probe script {}: {error}",
            path.display()
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_PAGE_PROBE_SCRIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            format!(
                "Could not read page probe script {}: {error}",
                path.display()
            )
        })?;
    if bytes.len() as u64 > MAX_PAGE_PROBE_SCRIPT_BYTES {
        return Err(format!(
            "Page probe script {} exceeds the {MAX_PAGE_PROBE_SCRIPT_BYTES}-byte limit",
            path.display()
        ));
    }
    String::from_utf8(bytes)
        .map_err(|error| format!("Page probe script {} is not UTF-8: {error}", path.display()))
}

fn default_page_visibility() -> bool {
    true
}

#[derive(Default)]
struct ConsoleDiagnostics {
    forwarded: usize,
    suppression_reported: bool,
}

fn flush_console_messages(
    runtime: &mut JsRuntime,
    diagnostics: &mut ConsoleDiagnostics,
) -> Result<(), String> {
    for message in runtime.take_console_messages() {
        if diagnostics.forwarded >= MAX_PAGE_CONSOLE_MESSAGES {
            if !diagnostics.suppression_reported {
                send(Update::Console {
                    level: "warn".into(),
                    message: format!(
                        "Page console output capped at {MAX_PAGE_CONSOLE_MESSAGES} messages"
                    ),
                })?;
                diagnostics.suppression_reported = true;
            }
            continue;
        }
        diagnostics.forwarded += 1;
        let level = match message.level {
            rix_js::runtime::ConsoleLogLevel::Log => "log",
            rix_js::runtime::ConsoleLogLevel::Warn => "warn",
            rix_js::runtime::ConsoleLogLevel::Error => "error",
            rix_js::runtime::ConsoleLogLevel::Info => "info",
            rix_js::runtime::ConsoleLogLevel::Debug => "debug",
        };
        let mut chars = message.message.chars();
        let mut text: String = chars
            .by_ref()
            .take(MAX_PAGE_CONSOLE_MESSAGE_CHARS)
            .collect();
        if chars.next().is_some() {
            text.push('…');
        }
        send(Update::Console {
            level: level.into(),
            message: text,
        })?;
    }
    Ok(())
}

fn evaluate_page_script(
    script: PageScript,
    execution_index: usize,
    script_element: Option<NodeId>,
    state: &Rc<RefCell<DomBridgeState>>,
    runtime: &mut JsRuntime,
    diagnostics: &mut ConsoleDiagnostics,
) -> Result<(), String> {
    let source_bytes = script.source.len();
    let diagnostic_url = rix_js::web_api::redact_resource_url(&script.source_url);
    send(Update::Busy)?;
    send(Update::ScriptStarted {
        index: execution_index,
        source_bytes,
        source_url: diagnostic_url.clone(),
    })?;
    state.borrow_mut().current_script = script_element;
    let (evaluation_result, evaluation_timing) = runtime.evaluate_with_phase_callback(
        &script.source,
        if script.source_url.is_empty() {
            None
        } else {
            Some(std::path::Path::new(&script.source_url))
        },
        |_| {},
    );
    state.borrow_mut().current_script = None;
    if let Err(error) = evaluation_result {
        send(Update::Error {
            message: script_error_message(&script, &error.to_string()),
        })?;
    }
    let promise_jobs_started = Instant::now();
    let promise_jobs_result = runtime.context_mut().run_jobs();
    let promise_jobs_ms = promise_jobs_started.elapsed().as_secs_f64() * 1000.0;
    if let Err(error) = promise_jobs_result {
        send(Update::Error {
            message: script_error_message(&script, &error.to_string()),
        })?;
    }
    if matches!(
        script.kind,
        PageScriptKind::ExternalClassic | PageScriptKind::DynamicClassic
    ) {
        dispatch_script_element_event(script_element, "load", runtime)?;
        if let Err(error) = runtime.context_mut().run_jobs() {
            send(Update::Error {
                message: script_error_message(&script, &error.to_string()),
            })?;
        }
    }
    flush_reported_exceptions(runtime.context_mut(), Some(&script))?;
    send(Update::ScriptTiming {
        timing: PageScriptTiming {
            index: execution_index,
            source_bytes,
            source_url: diagnostic_url.chars().take(256).collect(),
            evaluation_ms: evaluation_timing.total_ms,
            parse_ms: evaluation_timing.parse_ms,
            compile_ms: evaluation_timing.compile_ms,
            execute_ms: evaluation_timing.execute_ms,
            promise_jobs_ms,
        },
    })?;
    if std::env::var_os("RIX_WORKER_TRACE").is_some() && evaluation_timing.total_ms > 50.0 {
        eprintln!(
            "[rix-script-timing] eval={:.1}ms parse={:.1}ms compile={:.1}ms exec={:.1}ms promise_jobs={:.1}ms url={}",
            evaluation_timing.total_ms,
            evaluation_timing.parse_ms,
            evaluation_timing.compile_ms,
            evaluation_timing.execute_ms,
            promise_jobs_ms,
            diagnostic_url
        );
    }
    flush_console_messages(runtime, diagnostics)?;
    Ok(())
}

fn evaluate_page_module(
    script: PageScript,
    execution_index: usize,
    script_element: Option<NodeId>,
    module_loader: &BrowserModuleLoader,
    runtime: &mut JsRuntime,
    diagnostics: &mut ConsoleDiagnostics,
) -> Result<(), String> {
    let total_started = Instant::now();
    let source_bytes = script.source.len();
    let diagnostic_url = rix_js::web_api::redact_resource_url(&script.source_url);
    send(Update::Busy)?;
    send(Update::ScriptStarted {
        index: execution_index,
        source_bytes,
        source_url: diagnostic_url.clone(),
    })?;

    let parse_started = Instant::now();
    let source = boa_engine::Source::from_bytes(script.source.as_bytes())
        .with_path(std::path::Path::new(&script.source_url));
    let module = Module::parse(source, None, runtime.context_mut());
    let parse_ms = parse_started.elapsed().as_secs_f64() * 1000.0;
    let mut compile_ms = 0.0;
    let mut execute_ms = 0.0;
    let mut promise_jobs_ms = 0.0;
    let mut failure = None;
    match module {
        Err(error) => failure = Some(error.to_string()),
        Ok(module) => {
            module_loader.register_root(&script.source_url, module.clone());
            let loaded = module.load(runtime.context_mut());
            let loading_jobs_started = Instant::now();
            if let Err(error) = runtime.context_mut().run_jobs() {
                failure = Some(error.to_string());
            }
            promise_jobs_ms += loading_jobs_started.elapsed().as_secs_f64() * 1000.0;
            if failure.is_none() {
                match loaded.state() {
                    PromiseState::Fulfilled(_) => {
                        let compile_started = Instant::now();
                        if let Err(error) = module.link(runtime.context_mut()) {
                            failure = Some(error.to_string());
                        }
                        compile_ms = compile_started.elapsed().as_secs_f64() * 1000.0;
                    }
                    PromiseState::Rejected(reason) => {
                        failure = Some(js_value_diagnostic(&reason));
                    }
                    PromiseState::Pending => {
                        failure = Some("module graph load did not settle".into());
                    }
                }
            }
            if failure.is_none() {
                let execution_started = Instant::now();
                match module.evaluate(runtime.context_mut()) {
                    Ok(evaluated) => {
                        execute_ms = execution_started.elapsed().as_secs_f64() * 1000.0;
                        let promise_jobs_started = Instant::now();
                        if let Err(error) = runtime.context_mut().run_jobs() {
                            failure = Some(error.to_string());
                        }
                        promise_jobs_ms += promise_jobs_started.elapsed().as_secs_f64() * 1000.0;
                        if failure.is_none() {
                            match evaluated.state() {
                                PromiseState::Fulfilled(_) => {}
                                PromiseState::Rejected(reason) => {
                                    failure = Some(js_value_diagnostic(&reason));
                                }
                                PromiseState::Pending => {
                                    failure = Some(
                                        "top-level await is not yet integrated with the page event loop"
                                            .into(),
                                    );
                                }
                            }
                        }
                    }
                    Err(error) => {
                        execute_ms = execution_started.elapsed().as_secs_f64() * 1000.0;
                        failure = Some(error.to_string());
                    }
                }
            }
        }
    }

    if let Some(message) = failure {
        let diagnostic = rix_js::web_api::redact_diagnostic(&message);
        send(Update::Error {
            message: script_error_message(&script, &diagnostic),
        })?;
        send(Update::Console {
            level: "error".into(),
            message: format!(
                "Module script failed ({}): {diagnostic}",
                rix_js::web_api::redact_resource_url(&script.source_url)
            ),
        })?;
        dispatch_script_element_event(script_element, "error", runtime)?;
    } else {
        dispatch_script_element_event(script_element, "load", runtime)?;
    }
    if let Err(error) = runtime.context_mut().run_jobs() {
        send(Update::Error {
            message: script_error_message(&script, &error.to_string()),
        })?;
    }
    flush_reported_exceptions(runtime.context_mut(), Some(&script))?;
    send(Update::ScriptTiming {
        timing: PageScriptTiming {
            index: execution_index,
            source_bytes,
            source_url: diagnostic_url.chars().take(256).collect(),
            evaluation_ms: total_started.elapsed().as_secs_f64() * 1000.0,
            parse_ms,
            compile_ms,
            execute_ms,
            promise_jobs_ms,
        },
    })?;
    flush_console_messages(runtime, diagnostics)?;
    Ok(())
}

fn js_value_diagnostic(value: &boa_engine::JsValue) -> String {
    value
        .as_string()
        .map(|message| message.to_std_string_escaped())
        .unwrap_or_else(|| format!("{value:?}"))
}

fn dispatch_script_element_event(
    script_element: Option<NodeId>,
    event: &str,
    runtime: &mut JsRuntime,
) -> Result<(), String> {
    if let Some(script_element) = script_element {
        let properties = [
            ("bubbles", boa_engine::JsValue::new(false)),
            ("cancelable", boa_engine::JsValue::new(false)),
        ];
        report_page_event(script_element, event, Some(&properties), runtime)?;
    }
    Ok(())
}

fn queue_dynamic_classic_scripts(
    state: &Rc<RefCell<DomBridgeState>>,
    document_url: &str,
    tokens: &mut NodeTokens,
    started: &mut HashSet<NodeId>,
    async_modes: &mut HashMap<NodeToken, bool>,
    ordered_scripts: &mut VecDeque<NodeToken>,
    last_scanned_revision: &mut Option<u64>,
    remaining_request_budget: &mut usize,
    budget_exhausted_reported: &mut bool,
) -> Result<(), String> {
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let revision = document.script_revision();
    if *last_scanned_revision == Some(revision) {
        return Ok(());
    }
    *last_scanned_revision = Some(revision);
    let base_url = document_base_url(&document, document_url);
    for node in document.descendants(document.root()) {
        if started.contains(&node) || state.borrow().is_script_started(node) {
            continue;
        }
        let Some(NodeData::Element(element)) = document.get(node) else {
            continue;
        };
        if element.name.local.as_str() != "script" {
            continue;
        }
        if node_path(&document, node).is_none() {
            continue;
        }
        let Some(src) = element.get_attribute("src") else {
            continue;
        };
        if src.trim().is_empty() {
            continue;
        }
        if element.name.ns.as_str() != HTML_NAMESPACE
            || !is_classic_script_type(element.get_attribute("type").unwrap_or(""))
        {
            continue;
        }
        if *remaining_request_budget == 0 {
            if !*budget_exhausted_reported {
                started.insert(node);
                state.borrow_mut().mark_script_started(node);
                let target = tokens.token(node);
                let is_async = state.borrow().script_async(node);
                async_modes.insert(target, is_async);
                if !is_async {
                    ordered_scripts.push_back(target);
                }
                let source_url = resolve_external_resource(&base_url, src)
                    .map(|(url, _)| url)
                    .unwrap_or_else(|| src.to_string());
                send(Update::DynamicScriptRequest { target, source_url })?;
                *budget_exhausted_reported = true;
            }
            break;
        }
        started.insert(node);
        state.borrow_mut().mark_script_started(node);
        *remaining_request_budget = remaining_request_budget.saturating_sub(1);
        let target = tokens.token(node);
        let is_async = state.borrow().script_async(node);
        async_modes.insert(target, is_async);
        if !is_async {
            ordered_scripts.push_back(target);
        }
        let source_url = resolve_external_resource(&base_url, src)
            .map(|(url, _)| url)
            .unwrap_or_else(|| src.to_string());
        send(Update::DynamicScriptRequest { target, source_url })?;
    }
    Ok(())
}

struct DynamicScriptCompletion {
    source_url: String,
    result: Result<String, String>,
}

fn finish_dynamic_script_load(
    target: NodeToken,
    completion: DynamicScriptCompletion,
    tokens: &NodeTokens,
    state: &Rc<RefCell<DomBridgeState>>,
    runtime: &mut JsRuntime,
    console_diagnostics: &mut ConsoleDiagnostics,
    next_script_index: &mut usize,
) -> Result<(), String> {
    let script_element = tokens.by_token.get(&target).copied();
    let valid_script = script_element.is_some_and(|node| {
        matches!(
            state.borrow().document.borrow().get(node),
            Some(NodeData::Element(element))
                if element.name.ns.as_str() == HTML_NAMESPACE
                    && element.name.local.as_str() == "script"
        )
    });
    if !valid_script {
        return Ok(());
    }
    match completion.result {
        Ok(source) => {
            *next_script_index = next_script_index.saturating_add(1);
            evaluate_page_script(
                PageScript {
                    source,
                    element_index: 0,
                    kind: PageScriptKind::DynamicClassic,
                    source_url: completion.source_url,
                },
                *next_script_index,
                script_element,
                state,
                runtime,
                console_diagnostics,
            )?;
        }
        Err(message) => {
            send(Update::Console {
                level: "error".into(),
                message: format!(
                    "Failed to load dynamic script ({}): {message}",
                    rix_js::web_api::redact_resource_url(&completion.source_url)
                ),
            })?;
            if let Some(script_element) = script_element {
                let error_event_properties = [
                    ("bubbles", boa_engine::JsValue::new(false)),
                    ("cancelable", boa_engine::JsValue::new(false)),
                ];
                report_page_event(
                    script_element,
                    "error",
                    Some(&error_event_properties),
                    runtime,
                )?;
            }
        }
    }
    Ok(())
}

fn dispatch_page_event(
    target: NodeId,
    event: &str,
    properties: Option<&[(&str, boa_engine::JsValue)]>,
    runtime: &mut JsRuntime,
) -> Result<boa_engine::JsValue, String> {
    let result = dispatch_dom_event_to_js(target, event, properties, runtime.context_mut())
        .map_err(|error| error.to_string());
    flush_reported_exceptions(runtime.context_mut(), None)?;
    result
}

fn report_page_event(
    target: NodeId,
    event: &str,
    properties: Option<&[(&str, boa_engine::JsValue)]>,
    runtime: &mut JsRuntime,
) -> Result<(), String> {
    if let Err(message) = dispatch_page_event(target, event, properties, runtime) {
        send(Update::Error { message })?;
    }
    Ok(())
}

fn evaluate_page_lifecycle(runtime: &mut JsRuntime, source: &str) -> Result<(), String> {
    if let Err(error) = runtime.evaluate(source) {
        send(Update::Error {
            message: error.to_string(),
        })?;
    }
    flush_reported_exceptions(runtime.context_mut(), None)
}

fn dispatch_pending_scroll_event(
    host: &PageLayoutHost,
    state: &Rc<RefCell<DomBridgeState>>,
    runtime: &mut JsRuntime,
) -> Result<(), String> {
    if !host.take_scroll_event() {
        return Ok(());
    }
    let root = state.borrow().document.borrow().root();
    let props = [
        ("bubbles", boa_engine::JsValue::new(true)),
        ("cancelable", boa_engine::JsValue::new(false)),
    ];
    report_page_event(root, "scroll", Some(&props), runtime)
}

#[derive(Default)]
struct NodeTokens {
    by_node: HashMap<NodeId, NodeToken>,
    by_token: HashMap<NodeToken, NodeId>,
    next: NodeToken,
}

impl NodeTokens {
    fn token(&mut self, node: NodeId) -> NodeToken {
        if let Some(token) = self.by_node.get(&node) {
            return *token;
        }
        self.next = self.next.checked_add(1).expect("page node token exhausted");
        self.by_node.insert(node, self.next);
        self.by_token.insert(self.next, node);
        self.next
    }

    fn resolve(&self, doc: &Document, token: NodeToken) -> Option<NodeId> {
        self.by_token
            .get(&token)
            .copied()
            .filter(|node| node_path(doc, *node).is_some())
    }

    fn snapshot_nodes(&mut self, doc: &Document) -> Vec<SnapshotNode> {
        let mut nodes = Vec::new();
        let mut pending = vec![(doc.root(), None)];
        while let Some((node, parent)) = pending.pop() {
            let token = self.token(node);
            let data = match doc.get(node).expect("snapshot node exists") {
                NodeData::Document => SnapshotData::Document,
                NodeData::DocumentFragment => SnapshotData::DocumentFragment,
                NodeData::ShadowRoot {
                    host,
                    mode,
                    delegates_focus,
                    clonable,
                } => SnapshotData::ShadowRoot {
                    host: self.token(*host),
                    mode: mode.clone(),
                    delegates_focus: *delegates_focus,
                    clonable: *clonable,
                },
                NodeData::DocumentType {
                    name,
                    public_id,
                    system_id,
                } => SnapshotData::DocumentType {
                    name: name.clone(),
                    public_id: public_id.clone(),
                    system_id: system_id.clone(),
                },
                NodeData::Element(element) => {
                    if let Some(fragment) = element.template_contents {
                        pending.push((fragment, None));
                    }
                    if let Some(root) = element.shadow_root {
                        pending.push((root, None));
                    }
                    if let Some(doc_root) = element.content_document {
                        pending.push((doc_root, None));
                    }
                    SnapshotData::Element {
                        name: SnapshotName::from_name(&element.name),
                        attributes: element
                            .attributes
                            .iter()
                            .map(|(name, value)| (SnapshotName::from_name(name), value.clone()))
                            .collect(),
                        template_contents: element.template_contents.map(|node| self.token(node)),
                        canvas_bitmap: element.canvas_bitmap.clone(),
                        video_frame: element.video_frame.clone(),
                        shadow_root: element.shadow_root.map(|node| self.token(node)),
                        content_document: element.content_document.map(|node| self.token(node)),
                    }
                }
                NodeData::Text(value) => SnapshotData::Text(value.clone()),
                NodeData::Comment(value) => SnapshotData::Comment(value.clone()),
            };
            nodes.push(SnapshotNode {
                token,
                parent,
                data,
            });
            for child in doc.children(node).rev() {
                pending.push((child, Some(token)));
            }
        }
        nodes
    }

    fn focus_state(&mut self, doc: &Document, epoch: u64) -> FocusState {
        let node = doc
            .focused_node()
            .filter(|node| node_path(doc, *node).is_some());
        FocusState {
            epoch,
            revision: doc.revision(),
            target: node.map(|node| self.token(node)),
            selection: doc
                .text_selection()
                .filter(|(target, _, _)| Some(*target) == node)
                .map(|(_, anchor, cursor)| TextSelection { anchor, cursor }),
        }
    }
}

fn send_focus(
    state: &Rc<RefCell<DomBridgeState>>,
    tokens: &mut NodeTokens,
    epoch: u64,
) -> Result<(), String> {
    let doc = state.borrow().document.clone();
    let focus = tokens.focus_state(&doc.borrow(), epoch);
    send(Update::FocusChanged { focus })
}

fn snapshot(
    state: &Rc<RefCell<DomBridgeState>>,
    revision: &mut u64,
    tokens: &mut NodeTokens,
    epoch: u64,
) -> Result<bool, String> {
    let doc = state.borrow().document.clone();
    let doc = doc.borrow();
    if *revision != doc.revision() {
        *revision = doc.revision();
        let html = String::new();
        let mut dynamic_stylesheets = HashMap::new();
        for (node, css) in doc.all_dynamic_stylesheets() {
            dynamic_stylesheets.insert(tokens.token(*node), css.clone());
        }
        let mut adopted_stylesheets = HashMap::new();
        for (node, sheets) in doc.all_adopted_stylesheets() {
            adopted_stylesheets.insert(tokens.token(*node), sheets.clone());
        }
        let state = SnapshotState {
            nodes: tokens.snapshot_nodes(&doc),
            focus: tokens.focus_state(&doc, epoch),
            dynamic_stylesheets,
            adopted_stylesheets,
        };
        send(Update::FocusChanged { focus: state.focus })?;
        send(Update::Snapshot { html, state })?;
        return Ok(true);
    }
    Ok(false)
}

fn edit_control(
    state: &Rc<RefCell<DomBridgeState>>,
    runtime: &mut JsRuntime,
    node: NodeId,
    input: &KeyboardInput,
) -> Result<Option<(String, TextSelection)>, String> {
    let doc = state.borrow().document.clone();
    let (multiline, readonly, initial) = {
        let doc = doc.borrow();
        let Some(NodeData::Element(element)) = doc.get(node) else {
            return Ok(None);
        };
        if element.get_attribute("disabled").is_some() {
            return Ok(None);
        }
        let tag = element.name.local.as_str();
        let kind = element
            .get_attribute("type")
            .unwrap_or("text")
            .to_ascii_lowercase();
        let multiline = tag == "textarea";
        if !multiline
            && !(tag == "input"
                && matches!(
                    kind.as_str(),
                    "text" | "search" | "url" | "tel" | "email" | "password" | "number"
                ))
        {
            return Ok(None);
        }
        let value = element
            .get_attribute("value")
            .map(str::to_string)
            .unwrap_or_else(|| {
                if multiline {
                    doc.inner_text(node)
                } else {
                    String::new()
                }
            });
        (
            multiline,
            element.get_attribute("readonly").is_some(),
            value,
        )
    };
    let mut value = state
        .borrow()
        .input_values
        .get(&node)
        .cloned()
        .unwrap_or(initial);
    let mut selection = doc
        .borrow()
        .text_selection()
        .filter(|(target, _, _)| *target == node)
        .map(|(_, anchor, cursor)| TextSelection { anchor, cursor })
        .unwrap_or_else(|| TextSelection::collapsed(value.chars().count()));
    let result = edit_text(&mut value, &mut selection, input, multiline, readonly);
    if !result.handled {
        return Ok(None);
    }
    doc.borrow_mut()
        .set_text_selection(Some((node, selection.anchor, selection.cursor)));
    if result.changed {
        if let Some(NodeData::Element(element)) = doc.borrow_mut().get_mut(node) {
            element.set_attribute("value", &value);
        }
        state.borrow_mut().input_values.insert(node, value.clone());
        dispatch_page_event(node, "input", None, runtime)?;
        value = state
            .borrow()
            .input_values
            .get(&node)
            .cloned()
            .unwrap_or(value);
        if let Some((_, anchor, cursor)) = doc
            .borrow()
            .text_selection()
            .filter(|(target, _, _)| *target == node)
        {
            selection = TextSelection { anchor, cursor };
        }
    }
    Ok(Some((value, selection)))
}

fn apply_layout_change(host: &PageLayoutHost, change: LayoutChange) {
    match change {
        LayoutChange::Viewport { width, height } => {
            host.update(|inputs| inputs.viewport = (width, height))
        }
        LayoutChange::Scroll { x, y } => host.set_scroll(x, y),
        LayoutChange::ImageSize { url, width, height } => host.update(|inputs| {
            inputs.image_sizes.insert(url, (width, height));
        }),
        LayoutChange::Stylesheet { url, css } => host.update(|inputs| {
            inputs.stylesheets.insert(url, css);
        }),
    }
}

pub fn run_stdio_worker() -> Result<(), String> {
    if std::env::var_os("RIX_LOG").is_some() || std::env::var_os("RUST_LOG").is_some() {
        rix_core::logging::init_logging_to_stderr("warn");
    }
    std::thread::Builder::new()
        .name("page-runtime".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(run_worker_runtime)
        .map_err(|error| error.to_string())?
        .join()
        .map_err(|_| "page runtime thread panicked".to_string())?
}

fn run_worker_runtime() -> Result<(), String> {
    let mut input = BufReader::new(std::io::stdin());
    let Some(message) = read_message(&mut input).map_err(|error| error.to_string())? else {
        return Ok(());
    };
    let Request::Bootstrap {
        url,
        html,
        scripts,
        page_visible,
        dynamic_script_request_limit,
        navigation,
        layout,
        cookies,
        local_storage,
        theme,
    } = serde_json::from_slice(&message).map_err(|error| error.to_string())?
    else {
        return Err("page worker requires bootstrap first".into());
    };
    if let Some(ref t) = theme {
        let scheme = match t.as_str() {
            "light" => rix_css::ColorScheme::Light,
            _ => rix_css::ColorScheme::Dark,
        };
        rix_css::set_preferred_color_scheme(scheme);
        rix_css::clear_media_query_cache();
    }
    std::thread::spawn(|| {
        let _ = crate::page_layout_host::cached_system_fonts();
    });
    let (sender, requests) = mpsc::sync_channel(64);
    let module_waiters: ModuleLoadWaiters = Arc::new(Mutex::new(HashMap::new()));
    let input_module_waiters = module_waiters.clone();
    std::thread::spawn(move || {
        while let Ok(Some(message)) = read_message(&mut input) {
            let Ok(request) = serde_json::from_slice(&message) else {
                break;
            };
            match request {
                Request::ModuleLoadResult {
                    request_id,
                    source_url,
                    result,
                } => {
                    let waiter = input_module_waiters
                        .lock()
                        .ok()
                        .and_then(|mut waiters| waiters.remove(&request_id));
                    if let Some(waiter) = waiter {
                        let _ = waiter.send(ModuleLoadResponse { source_url, result });
                    }
                }
                request => {
                    if sender.send(request).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let origin = rix_core::url::RixUrl::parse(&url).ok();
    let layout_host = Rc::new(PageLayoutHost::new(url.clone(), layout));
    let module_loader = Rc::new(BrowserModuleLoader::new(
        url.clone(),
        dynamic_script_request_limit,
        module_waiters,
    ));
    let mut runtime = JsRuntime::new_with_module_loader(
        JsRuntimeConfig::for_page_worker(url.clone()),
        module_loader.clone(),
    );
    let shared_document = Rc::new(RefCell::new(Document::new()));
    let mut parser_stream = HtmlParserStream::with_document(shared_document.clone());
    parser_stream.feed(&html);

    let initial_scheme = if let Some(ref t) = theme {
        match t.as_str() {
            "light" => rix_css::ColorScheme::Light,
            _ => rix_css::ColorScheme::Dark,
        }
    } else {
        rix_css::preferred_color_scheme()
    };
    if initial_scheme == rix_css::ColorScheme::Dark {
        let mut doc = shared_document.borrow_mut();
        let root = doc.root();
        if let Some(html_id) = doc.query_selector(root, "html") {
            if let Some(NodeData::Element(elem)) = doc.get_mut(html_id) {
                if (url.contains("youtube.com") || url.contains("youtu.be")) && !elem.has_attribute("dark") {
                    elem.set_attribute("dark", "");
                }
            }
        }
    }

    let mut dom_state = DomBridgeState::with_shared_document(shared_document.clone());
    dom_state.page_visible = page_visible;
    let state = Rc::new(RefCell::new(dom_state));
    let mut tokens = NodeTokens::default();
    let mut started_script_elements = HashSet::<NodeId>::new();
    let mut dynamic_script_async_modes = HashMap::<NodeToken, bool>::new();
    let mut ordered_dynamic_scripts = VecDeque::<NodeToken>::new();
    let mut ordered_dynamic_script_completions =
        HashMap::<NodeToken, DynamicScriptCompletion>::new();
    let mut last_dynamic_script_scan_revision = None;
    let mut remaining_dynamic_script_requests = dynamic_script_request_limit;
    let mut dynamic_script_budget_exhausted_reported = false;
    let mut next_script_index = 0usize;
    let mut focus_epoch = 0;
    let mut console_diagnostics = ConsoleDiagnostics::default();
    let mut resource_records_sent = 0;
    let mut resource_trace_truncated_reported = false;
    let mut last_pending_network_count = None;
    let web = Rc::new(RefCell::new(WebApiState::new(origin)));
    if !cookies.is_empty() {
        if let Some(store) = &web.borrow().cookie_store {
            let _ = store.set_cookies(cookies);
        }
    }
    if !local_storage.is_empty() {
        web.borrow_mut().local_storage.extend(local_storage);
    }
    register_web_apis(runtime.context_mut(), web.clone()).map_err(|error| error.to_string())?;
    let mut events = EventLoop::new(web.clone());
    if let Some(navigation) = navigation {
        rix_js::web_api::set_navigation_performance(
            runtime.context_mut(),
            navigation.timing,
            navigation.elapsed_ms,
        )
        .map_err(|error| error.to_string())?;
    }
    register_dom_bridge(runtime.context_mut(), state.clone()).map_err(|error| error.to_string())?;
    register_layout_api(runtime.context_mut(), layout_host.clone())
        .map_err(|error| error.to_string())?;
    let initial_root = state.borrow().document.borrow().root();
    if let Err(error) =
        rix_js::dom_bridge::set_document_ready_state(runtime.context_mut(), initial_root, "loading")
    {
        send(Update::Error {
            message: error.to_string(),
        })?;
    }
    flush_console_messages(&mut runtime, &mut console_diagnostics)?;
    rix_js::web_api::mark_navigation_milestone(
        runtime.context_mut(),
        rix_js::web_api::NavigationMilestone::DomLoading,
    );

    if let Some(source) = read_page_probe_script()? {
        next_script_index = next_script_index.saturating_add(1);
        evaluate_page_script(
            PageScript {
                source,
                element_index: usize::MAX,
                kind: PageScriptKind::DiagnosticProbe,
                source_url: String::new(),
            },
            next_script_index,
            None,
            &state,
            &mut runtime,
            &mut console_diagnostics,
        )?;
        flush_resource_loads(
            &web,
            &mut resource_records_sent,
            &mut resource_trace_truncated_reported,
        )?;
        flush_cookie_updates(&web)?;
        flush_storage_mutations(&web)?;
        flush_page_requests(&web)?;
        queue_dynamic_classic_scripts(
            &state,
            &url,
            &mut tokens,
            &mut started_script_elements,
            &mut dynamic_script_async_modes,
            &mut ordered_dynamic_scripts,
            &mut last_dynamic_script_scan_revision,
            &mut remaining_dynamic_script_requests,
            &mut dynamic_script_budget_exhausted_reported,
        )?;
        flush_scroll_request(&layout_host)?;
        send(Update::Idle)?;
    }

    let mut scripts_by_index: HashMap<usize, PageScript> = scripts
        .into_iter()
        .map(|s| (s.element_index, s))
        .collect();
    let mut deferred_scripts = Vec::new();
    let mut script_element_counter = 0usize;

    loop {
        match parser_stream.step() {
            ParserStreamStep::Script(node_id) => {
                let current_index = script_element_counter;
                script_element_counter += 1;
                started_script_elements.insert(node_id);
                state.borrow_mut().mark_script_started(node_id);

                if let Some(script) = scripts_by_index.remove(&current_index) {
                    let doc = state.borrow().document.clone();
                    let is_defer = {
                        let doc = doc.borrow();
                        if let Some(NodeData::Element(elem)) = doc.get(node_id) {
                            elem.has_attribute("defer")
                        } else {
                            false
                        }
                    };
                    let is_module = matches!(
                        script.kind,
                        PageScriptKind::InlineModule | PageScriptKind::ExternalModule
                    );
                    if is_defer || is_module {
                        deferred_scripts.push((script, Some(node_id), is_module));
                    } else {
                        next_script_index = next_script_index.saturating_add(1);
                        let execution_index = next_script_index;
                        evaluate_page_script(
                            script,
                            execution_index,
                            Some(node_id),
                            &state,
                            &mut runtime,
                            &mut console_diagnostics,
                        )?;
                        flush_resource_loads(
                            &web,
                            &mut resource_records_sent,
                            &mut resource_trace_truncated_reported,
                        )?;
                        flush_cookie_updates(&web)?;
                        flush_storage_mutations(&web)?;
                        flush_page_requests(&web)?;
                        queue_dynamic_classic_scripts(
                            &state,
                            &url,
                            &mut tokens,
                            &mut started_script_elements,
                            &mut dynamic_script_async_modes,
                            &mut ordered_dynamic_scripts,
                            &mut last_dynamic_script_scan_revision,
                            &mut remaining_dynamic_script_requests,
                            &mut dynamic_script_budget_exhausted_reported,
                        )?;
                        flush_scroll_request(&layout_host)?;
                        send(Update::Idle)?;
                    }
                }
            }
            ParserStreamStep::Done => {
                parser_stream.finish();
                break;
            }
        }
    }

    let mut remaining_scripts: Vec<(usize, PageScript)> = scripts_by_index.into_iter().collect();
    remaining_scripts.sort_by_key(|(idx, _)| *idx);
    for (_, script) in remaining_scripts {
        let is_module = matches!(
            script.kind,
            PageScriptKind::InlineModule | PageScriptKind::ExternalModule
        );
        deferred_scripts.push((script, None, is_module));
    }

    tokens.snapshot_nodes(&state.borrow().document.borrow());
    let mut revision = 0u64;

    for (script, script_element, is_module) in deferred_scripts {
        next_script_index = next_script_index.saturating_add(1);
        let execution_index = next_script_index;
        if is_module {
            evaluate_page_module(
                script,
                execution_index,
                script_element,
                &module_loader,
                &mut runtime,
                &mut console_diagnostics,
            )?;
        } else {
            evaluate_page_script(
                script,
                execution_index,
                script_element,
                &state,
                &mut runtime,
                &mut console_diagnostics,
            )?;
        }
        flush_resource_loads(
            &web,
            &mut resource_records_sent,
            &mut resource_trace_truncated_reported,
        )?;
        flush_cookie_updates(&web)?;
        flush_storage_mutations(&web)?;
        flush_page_requests(&web)?;
        queue_dynamic_classic_scripts(
            &state,
            &url,
            &mut tokens,
            &mut started_script_elements,
            &mut dynamic_script_async_modes,
            &mut ordered_dynamic_scripts,
            &mut last_dynamic_script_scan_revision,
            &mut remaining_dynamic_script_requests,
            &mut dynamic_script_budget_exhausted_reported,
        )?;
        flush_scroll_request(&layout_host)?;
        let _ = snapshot(&state, &mut revision, &mut tokens, focus_epoch)?;
        send(Update::Idle)?;
    }
    send(Update::Busy)?;
    let root = state.borrow().document.borrow().root();
    send_task_started("set document.readyState to interactive")?;
    if let Err(error) =
        rix_js::dom_bridge::set_document_ready_state(runtime.context_mut(), root, "interactive")
    {
        send(Update::Error {
            message: error.to_string(),
        })?;
    }
    rix_js::web_api::mark_navigation_milestone(
        runtime.context_mut(),
        rix_js::web_api::NavigationMilestone::DomInteractive,
    );
    rix_js::web_api::mark_navigation_milestone(
        runtime.context_mut(),
        rix_js::web_api::NavigationMilestone::DomContentLoadedEventStart,
    );
    send_task_started("dispatch DOMContentLoaded")?;
    let dom_content_loaded_props = [
        ("bubbles", boa_engine::JsValue::new(true)),
        ("cancelable", boa_engine::JsValue::new(false)),
    ];
    report_page_event(
        root,
        "DOMContentLoaded",
        Some(&dom_content_loaded_props),
        &mut runtime,
    )?;
    rix_js::web_api::mark_navigation_milestone(
        runtime.context_mut(),
        rix_js::web_api::NavigationMilestone::DomContentLoadedEventEnd,
    );
    send_task_started("set document.readyState to complete")?;
    if let Err(error) =
        rix_js::dom_bridge::set_document_ready_state(runtime.context_mut(), root, "complete")
    {
        send(Update::Error {
            message: error.to_string(),
        })?;
    }
    rix_js::web_api::mark_navigation_milestone(
        runtime.context_mut(),
        rix_js::web_api::NavigationMilestone::DomComplete,
    );
    rix_js::web_api::mark_navigation_milestone(
        runtime.context_mut(),
        rix_js::web_api::NavigationMilestone::LoadEventStart,
    );
    send_task_started("dispatch window load")?;
    evaluate_page_lifecycle(&mut runtime, "window.dispatchEvent(new Event('load'));")?;
    send_task_started("deliver rendering observer notifications")?;
    if let Err(error) = rix_js::dom_bridge::run_layout_observers(runtime.context_mut()) {
        send(Update::Error {
            message: error.to_string(),
        })?;
    }
    if let Err(error) = runtime.context_mut().run_jobs() {
        send(Update::Error {
            message: error.to_string(),
        })?;
    }
    flush_reported_exceptions(runtime.context_mut(), None)?;
    rix_js::web_api::mark_navigation_milestone(
        runtime.context_mut(),
        rix_js::web_api::NavigationMilestone::LoadEventEnd,
    );
    send(Update::TaskStarted {
        label: "dispatch pending document scroll event".into(),
    })?;
    dispatch_pending_scroll_event(&layout_host, &state, &mut runtime)?;
    flush_console_messages(&mut runtime, &mut console_diagnostics)?;
    flush_resource_loads(
        &web,
        &mut resource_records_sent,
        &mut resource_trace_truncated_reported,
    )?;
    flush_media_playback(&mut runtime)?;
    flush_media_samples(&mut runtime)?;
    flush_cookie_updates(&web)?;
    flush_storage_mutations(&web)?;
    flush_page_requests(&web)?;
    flush_pending_network_count(&web, &mut last_pending_network_count)?;
    queue_dynamic_classic_scripts(
        &state,
        &url,
        &mut tokens,
        &mut started_script_elements,
        &mut dynamic_script_async_modes,
        &mut ordered_dynamic_scripts,
        &mut last_dynamic_script_scan_revision,
        &mut remaining_dynamic_script_requests,
        &mut dynamic_script_budget_exhausted_reported,
    )?;
    flush_scroll_request(&layout_host)?;
    let _ = snapshot(&state, &mut revision, &mut tokens, focus_epoch)?;
    send(Update::Idle)?;
    if events.take_animation_frame_request() {
        send(Update::AnimationFrameRequested)?;
    }
    send(Update::Ready)?;
    let mut deferred_request = None;
    let trace_enabled = std::env::var_os("RIX_WORKER_TRACE").is_some();
    let mut trace_iterations = 0usize;
    let trace_started = Instant::now();
    let mut last_snapshot_time = Instant::now();
    loop {
        let wait = if layout_host.has_pending_scroll_event() {
            Duration::ZERO
        } else {
            events
                .next_deadline()
                .map(|deadline| deadline.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::from_secs(60))
        };
        if wait > Duration::ZERO {
            if snapshot(&state, &mut revision, &mut tokens, focus_epoch)? {
                last_snapshot_time = Instant::now();
            }
        }
        if trace_enabled && trace_iterations < 1500 {
            trace_iterations += 1;
            let (microtasks, jobs, engine_jobs, timers, fetches) = events.debug_pending_counts();
            eprintln!(
                "[rix-trace] iter {trace_iterations} t={}ms microtasks={microtasks} jobs={jobs} engine_jobs={engine_jobs} timers={timers} fetches={fetches} wait_ms={}",
                trace_started.elapsed().as_millis(),
                wait.as_millis()
            );
        }
        flush_media_playback(&mut runtime)?;
        flush_media_samples(&mut runtime)?;
        let request = if events.has_pending_microtasks() {
            if deferred_request.is_none() {
                match requests.try_recv() {
                    Ok(request) => deferred_request = Some(request),
                    Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout)
        } else if let Some(request) = deferred_request.take() {
            Ok(request)
        } else {
            requests.recv_timeout(wait)
        };
        let handled_request = matches!(request, Ok(_));
        let mut frame_opportunity_in_progress = false;
        match request {
            Ok(Request::Focus {
                target,
                epoch,
                selection,
            }) => {
                send(Update::Busy)?;
                send(Update::TaskStarted {
                    label: "focus event dispatch".into(),
                })?;
                focus_epoch = epoch;
                let doc = state.borrow().document.clone();
                let old = doc.borrow().focused_node();
                let node = target.and_then(|token| tokens.resolve(&doc.borrow(), token));
                doc.borrow_mut().set_focused(node, false);
                doc.borrow_mut().set_text_selection(
                    node.map(|node| (node, selection.anchor, selection.cursor)),
                );
                if old != node {
                    if let Some(old) = old {
                        report_page_event(old, "blur", None, &mut runtime)?;
                        report_page_event(old, "focusout", None, &mut runtime)?;
                    }
                    if let Some(node) = node {
                        report_page_event(node, "focus", None, &mut runtime)?;
                        report_page_event(node, "focusin", None, &mut runtime)?;
                    }
                }
            }
            Ok(Request::SetPageVisibility { visible }) => {
                let changed = {
                    let mut state = state.borrow_mut();
                    let changed = state.page_visible != visible;
                    state.page_visible = visible;
                    changed
                };
                if changed {
                    send(Update::Busy)?;
                    send(Update::TaskStarted {
                        label: "dispatch visibilitychange".into(),
                    })?;
                    let root = state.borrow().document.borrow().root();
                    report_page_event(root, "visibilitychange", None, &mut runtime)?;
                }
            }
            Ok(Request::AnimationFrameOpportunity { timestamp_ms }) => {
                send(Update::Busy)?;
                let mut task_start_error = None;
                events.run_rendering_step_with_progress(
                    timestamp_ms,
                    runtime.context_mut(),
                    |phase| {
                        if task_start_error.is_none() {
                            task_start_error = send(Update::TaskStarted {
                                label: format!("page {phase}"),
                            })
                            .err();
                        }
                    },
                );
                if let Some(error) = task_start_error {
                    return Err(error);
                }
                frame_opportunity_in_progress = true;
            }
            Ok(Request::Event {
                id,
                target,
                event,
                keyboard,
                value,
            }) => {
                send(Update::Busy)?;
                send(Update::TaskStarted {
                    label: "input event dispatch".into(),
                })?;
                let node = {
                    let doc = state.borrow().document.clone();
                    let doc = doc.borrow();
                    if keyboard.is_some() {
                        Some(
                            doc.focused_node()
                                .filter(|node| node_path(&doc, *node).is_some())
                                .unwrap_or_else(|| doc.root()),
                        )
                    } else {
                        tokens.resolve(&doc, target)
                    }
                };
                let mut allowed = false;
                if let Some(node) = node {
                    if let Some(value) = value {
                        let doc = state.borrow().document.clone();
                        if let Some(NodeData::Element(element)) = doc.borrow_mut().get_mut(node) {
                            element.set_attribute("value", &value);
                        }
                        state.borrow_mut().input_values.insert(node, value);
                    }
                    if keyboard.is_some() && node != state.borrow().document.borrow().root() {
                        state
                            .borrow()
                            .document
                            .borrow_mut()
                            .set_focused(Some(node), true);
                    }
                    let props = keyboard.as_ref().map(|input| {
                        let mut props = vec![
                            ("key", boa_engine::js_string!(input.key.as_str()).into()),
                            ("code", boa_engine::js_string!(input.code.as_str()).into()),
                            ("ctrlKey", boa_engine::JsValue::new(input.ctrl)),
                            ("altKey", boa_engine::JsValue::new(input.alt)),
                            ("shiftKey", boa_engine::JsValue::new(input.shift)),
                            ("metaKey", boa_engine::JsValue::new(input.meta)),
                        ];
                        if matches!(event.as_str(), "keydown" | "keyup" | "keypress") {
                            let (key_code, char_code, which) =
                                rix_js::dom_bridge::legacy_key::legacy_key_values(
                                    event.as_str(),
                                    &input.key,
                                    &input.code,
                                );
                            props.push(("keyCode", boa_engine::JsValue::new(key_code)));
                            props.push(("charCode", boa_engine::JsValue::new(char_code)));
                            props.push(("which", boa_engine::JsValue::new(which)));
                        }
                        props
                    });
                    allowed =
                        match dispatch_page_event(node, &event, props.as_deref(), &mut runtime) {
                            Ok(result) => result.to_boolean(),
                            Err(message) => {
                                send(Update::Error { message })?;
                                false
                            }
                        };
                }
                send_focus(&state, &mut tokens, focus_epoch)?;
                if allowed {
                    let focused = state.borrow().document.borrow().focused_node();
                    if let (Some(node), Some(input)) = (focused, keyboard.as_ref()) {
                        let edited = match edit_control(&state, &mut runtime, node, input) {
                            Ok(edited) => edited,
                            Err(message) => {
                                send(Update::Error { message })?;
                                None
                            }
                        };
                        if let Some((value, selection)) = edited {
                            send_focus(&state, &mut tokens, focus_epoch)?;
                            send(Update::TextEdited {
                                id,
                                target: tokens.token(node),
                                focus_epoch,
                                value,
                                selection,
                            })?;
                        }
                    }
                }
                let default_target = if keyboard.is_some() {
                    state.borrow().document.borrow().focused_node()
                } else {
                    node
                };
                send(Update::EventHandled {
                    id,
                    allowed,
                    target: default_target.map(|node| tokens.token(node)),
                    focus_epoch,
                })?;
            }
            Ok(Request::ScriptLoadResult {
                target,
                source_url,
                result,
            }) => {
                send(Update::Busy)?;
                send(Update::TaskStarted {
                    label: "dynamic script load completion".into(),
                })?;
                let completion = DynamicScriptCompletion { source_url, result };
                if dynamic_script_async_modes
                    .get(&target)
                    .copied()
                    .unwrap_or(true)
                {
                    dynamic_script_async_modes.remove(&target);
                    finish_dynamic_script_load(
                        target,
                        completion,
                        &tokens,
                        &state,
                        &mut runtime,
                        &mut console_diagnostics,
                        &mut next_script_index,
                    )?;
                } else {
                    ordered_dynamic_script_completions.insert(target, completion);
                }
                while let Some(next_target) = ordered_dynamic_scripts.front().copied() {
                    let Some(completion) = ordered_dynamic_script_completions.remove(&next_target)
                    else {
                        break;
                    };
                    ordered_dynamic_scripts.pop_front();
                    dynamic_script_async_modes.remove(&next_target);
                    finish_dynamic_script_load(
                        next_target,
                        completion,
                        &tokens,
                        &state,
                        &mut runtime,
                        &mut console_diagnostics,
                        &mut next_script_index,
                    )?;
                }
            }
            Ok(Request::Layout(change)) => {
                let is_viewport = matches!(change, LayoutChange::Viewport { .. });
                apply_layout_change(&layout_host, change);
                if is_viewport {
                    {
                        let doc_rc = state.borrow().document.clone();
                        let mut doc = doc_rc.borrow_mut();
                        doc.bump_style_revision();
                        doc.bump_revision();
                    }
                    let _ = runtime.evaluate("if (typeof window !== 'undefined' && typeof window.dispatchEvent === 'function') window.dispatchEvent(new Event('resize'));");
                    let _ = rix_js::dom_bridge::notify_match_media(runtime.context_mut());
                }
                send(Update::Busy)?;
                send(Update::TaskStarted {
                    label: if layout_host.has_pending_scroll_event() {
                        "layout scroll event dispatch".into()
                    } else if is_viewport {
                        "layout viewport resize".into()
                    } else {
                        "layout observer checkpoint".into()
                    },
                })?;
            }
            Ok(Request::SetTheme { theme }) => {
                let scheme = match theme.as_str() {
                    "light" => rix_css::ColorScheme::Light,
                    _ => rix_css::ColorScheme::Dark,
                };
                rix_css::set_preferred_color_scheme(scheme);
                rix_css::clear_media_query_cache();
                {
                    let doc_rc = state.borrow().document.clone();
                    let mut doc = doc_rc.borrow_mut();
                    let root = doc.root();
                    if let Some(html_id) = doc.query_selector(root, "html") {
                        if let Some(NodeData::Element(elem)) = doc.get_mut(html_id) {
                            if url.contains("youtube.com") || url.contains("youtu.be") {
                                if scheme == rix_css::ColorScheme::Dark {
                                    elem.set_attribute("dark", "");
                                } else {
                                    elem.remove_attribute("dark");
                                }
                            }
                        }
                    }
                    doc.bump_style_revision();
                    doc.bump_revision();
                }
                let _ = rix_js::dom_bridge::notify_match_media(runtime.context_mut());
                send(Update::Busy)?;
                send(Update::TaskStarted {
                    label: "theme changed notification".into(),
                })?;
            }
            Ok(Request::TraverseHistory { amount }) => {
                send(Update::Busy)?;
                send(Update::TaskStarted {
                    label: "history traversal".into(),
                })?;
                match rix_js::web_api::traverse_session_history(runtime.context_mut(), amount) {
                    Ok(true) => {}
                    Ok(false) => {
                        send(Update::HistoryTraversalUnavailable { amount })?;
                    }
                    Err(error) => {
                        send(Update::Error {
                            message: error.to_string(),
                        })?;
                    }
                }
            }
            Ok(_) => continue,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let mut task_start_error = None;
        let mut step_phase: Option<&'static str> = None;
        let step_started = Instant::now();
        let stepped = events.step_with_progress(runtime.context_mut(), |phase| {
            step_phase = Some(phase);
            if task_start_error.is_none() {
                task_start_error = send(Update::TaskStarted {
                    label: format!("page {phase}"),
                })
                .err();
            }
        });
        let step_elapsed = step_started.elapsed();
        if trace_enabled && trace_iterations >= 1 && trace_iterations <= 1500 {
            let (microtasks, jobs, engine_jobs, timers, fetches) = events.debug_pending_counts();
            eprintln!(
                "[rix-trace]   step={stepped} microtasks={microtasks} jobs={jobs} engine_jobs={engine_jobs} timers={timers} fetches={fetches}"
            );
        }
        if let Some(error) = task_start_error {
            return Err(error);
        }
        let tail_started = Instant::now();
        flush_resource_loads(
            &web,
            &mut resource_records_sent,
            &mut resource_trace_truncated_reported,
        )?;
        flush_cookie_updates(&web)?;
        flush_storage_mutations(&web)?;
        flush_page_requests(&web)?;
        flush_pending_network_count(&web, &mut last_pending_network_count)?;
        for message in events.take_task_errors() {
            send(Update::Error { message })?;
        }
        dispatch_pending_scroll_event(&layout_host, &state, &mut runtime)?;
        queue_dynamic_classic_scripts(
            &state,
            &url,
            &mut tokens,
            &mut started_script_elements,
            &mut dynamic_script_async_modes,
            &mut ordered_dynamic_scripts,
            &mut last_dynamic_script_scan_revision,
            &mut remaining_dynamic_script_requests,
            &mut dynamic_script_budget_exhausted_reported,
        )?;
        flush_console_messages(&mut runtime, &mut console_diagnostics)?;
        flush_scroll_request(&layout_host)?;
        let has_immediate_work = events.has_pending_microtasks()
            || events
                .next_deadline()
                .map_or(false, |deadline| deadline <= Instant::now());
        let throttle_snapshot = has_immediate_work
            && !frame_opportunity_in_progress
            && last_snapshot_time.elapsed() < Duration::from_millis(32);
        if !throttle_snapshot {
            if snapshot(&state, &mut revision, &mut tokens, focus_epoch)? {
                last_snapshot_time = Instant::now();
            }
        }
        if frame_opportunity_in_progress {
            send(Update::AnimationFrameComplete)?;
        }
        if events.take_animation_frame_request() {
            send(Update::AnimationFrameRequested)?;
        }
        if trace_enabled && trace_iterations >= 1 && trace_iterations <= 1500 {
            eprintln!("[rix-trace]   iter-tail {trace_iterations}");
        }
        let tail_elapsed = tail_started.elapsed();
        if trace_enabled
            && (step_elapsed >= Duration::from_millis(50) || tail_elapsed >= Duration::from_millis(50))
        {
            let (microtasks, jobs, engine_jobs, timers, fetches) = events.debug_pending_counts();
            eprintln!(
                "[rix-trace] slow iteration step={}ms tail={}ms phase={step_phase:?} \
                 microtasks={microtasks} jobs={jobs} engine_jobs={engine_jobs} \
                 timers={timers} fetches={fetches}",
                step_elapsed.as_millis(),
                tail_elapsed.as_millis()
            );
        }
        if stepped > 0 || handled_request {
            send(Update::Idle)?;
        }
    }
    Ok(())
}
