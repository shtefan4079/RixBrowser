#![deny(unsafe_code)]

use rix_app::{version_string, AppConfig, CliArgs, DEFAULT_HEIGHT, DEFAULT_WIDTH};
use rix_chrome::customization::{CustomizationConfig, CustomizationManager};
use rix_chrome::server::ControlMessage;
use rix_chrome::shell::BrowserShell;
use rix_ipc::protocol::IpcInputEvent;
use rix_render::VelloRenderer;
use rix_storage::{SecretStoreError, StorageEngine};
#[cfg(feature = "software")]
use softbuffer::{Context, Surface};
use std::env;
#[cfg(feature = "software")]
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn, Level};
use tracing_subscriber::FmtSubscriber;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--page-runtime-worker") {
        return rix_chrome::page_worker::run_stdio_worker().map_err(Into::into);
    }
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    info!("Starting {}", version_string());

    let raw_args: Vec<String> = env::args().collect();
    let cli = CliArgs::parse_from(raw_args.clone()).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });

    if cli.show_help {
        println!("{}", CliArgs::usage());
        return Ok(());
    }

    if cli.show_version {
        println!("{}", version_string());
        return Ok(());
    }

    let config_path = AppConfig::default_path();
    let file_config = AppConfig::load_or_default(&config_path).unwrap_or_else(|e| {
        eprintln!("warning: {e}; falling back to defaults");
        AppConfig::default()
    });
    let config = cli.apply_to_config(file_config);
    if let Err(e) = config.save_to_path(&config_path) {
        eprintln!("warning: cannot persist config: {e}");
    }

    let no_display = env::var("DISPLAY").is_err() && env::var("WAYLAND_DISPLAY").is_err();
    let is_headless = cli.headless || config.headless || no_display;
    let control_port = config.control_port;

    let (storage, secret_warning) =
        StorageEngine::open_with_system_secrets(&AppConfig::data_dir().join("profile.redb"))
            .await?;
    if let Some(warning) = secret_warning {
        warn!("{warning}");
        eprintln!("{warning}");
    }

    let (tx, mut rx) = mpsc::channel(64);
    rix_chrome::server::start_control_server(tx, control_port);

    if is_headless {
        info!("Running RixBrowser in headless automation mode");
        let mut shell = boot_shell_with_config(&config, storage.clone(), secret_warning)?;
        if !config.homepage.is_empty() && config.homepage != "about:blank" {
            let target = config.homepage.clone();
            shell.navigate_active_tab(&target).await;
        }
        println!("RixBrowser Automation Server listening on http://127.0.0.1:{control_port}");
        let mut navigation_poll = tokio::time::interval(std::time::Duration::from_millis(16));
        loop {
            tokio::select! {
                message = rx.recv() => match message {
                    Some(msg) => shell.handle_control_message(msg).await,
                    None => break,
                },
                _ = navigation_poll.tick() => {
                    shell.poll_navigation_completions();
                    shell.step_event_loop();
                }
            }
        }
        return Ok(());
    }

    let startup_start = std::time::Instant::now();
    info!("Initializing native desktop window via winit");
    let mut shell = boot_shell_with_config(&config, storage.clone(), secret_warning)?;
    eprintln!("[{:.3}] boot_shell took {}ms", startup_start.elapsed().as_secs_f64(), startup_start.elapsed().as_millis());
    let t_nav = std::time::Instant::now();
    if !config.homepage.is_empty() && config.homepage != "about:blank" {
        shell.start_navigation(1, &config.homepage);
    }
    eprintln!("[{:.3}] start_navigation took {}ms", startup_start.elapsed().as_secs_f64(), t_nav.elapsed().as_millis());
    println!(
        "RixBrowser UI Shell initialized on {}x{} surface.",
        config.window_width as u32, config.window_height as u32
    );
    println!("Automation API active on http://127.0.0.1:{control_port}");
    println!("Opening native OS window (Wayland/X11)...");

    let t_el = std::time::Instant::now();
    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    eprintln!("[{:.3}] EventLoop::build took {}ms", startup_start.elapsed().as_secs_f64(), t_el.elapsed().as_millis());
    let (gpu_tx, gpu_rx) = std::sync::mpsc::sync_channel::<VelloRenderer>(1);
    std::thread::Builder::new()
        .name("gpu-warmup".into())
        .spawn(move || {
            let t0 = std::time::Instant::now();
            let renderer = VelloRenderer::new();
            eprintln!("[rix-timing] Background VelloRenderer::new took {}ms", t0.elapsed().as_millis());
            let _ = gpu_tx.send(renderer);
        })
        .ok();

    let proxy = event_loop.create_proxy();
    let mut app = RixBrowserApp {
        shell: Some(shell),
        gpu: None,
        gpu_rx: Some(gpu_rx),
        #[cfg(feature = "software")]
        context: None,
        #[cfg(feature = "software")]
        surface: None,
        window: None,
        mouse_pos: (0.0, 0.0),
        mouse_pressed: false,
        modifiers: ModifiersState::default(),
    };
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            match proxy.send_event(AppEvent::Control(message)) {
                Ok(()) => tracing::debug!("control message forwarded"),
                Err(error) => {
                    tracing::error!(%error, "control message forward failed");
                    break;
                }
            }
        }
    });

    eprintln!("[{:.3}] calling event_loop.run_app", startup_start.elapsed().as_secs_f64());
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn boot_shell_with_config(
    config: &AppConfig,
    storage: StorageEngine,
    secret_warning: Option<SecretStoreError>,
) -> Result<BrowserShell, Box<dyn std::error::Error>> {
    let customization = CustomizationManager::new(CustomizationConfig {
        tab_orientation: config.tab_orientation(),
        theme_mode: config.theme_mode(),
        custom_css_path: CustomizationConfig::default().custom_css_path,
    });
    let mut shell = BrowserShell::new_with_customization(
        config.window_width,
        config.window_height,
        customization,
    )?;
    shell.set_cookie_persistence_notice(match secret_warning {
        None => "Protected cookies",
        Some(SecretStoreError::Locked) => "Clavis locked · temporary cookies",
        Some(_) => "Cookies stay until this browser closes",
    });
    shell.storage = Some(storage.clone());
    shell.http_client = rix_net::HttpClient::new(Some(storage))?;
    shell.search_engine_template = config.search_engine_template.clone();
    shell.homepage = config.homepage.clone();
    shell.page_worker_command = Some(rix_chrome::page_worker::WorkerCommand {
        executable: std::env::current_exe()?,
        arguments: Vec::new(),
    });
    Ok(shell)
}

enum AppEvent {
    Control(ControlMessage),
}

struct RixBrowserApp {
    shell: Option<BrowserShell>,
    gpu: Option<VelloRenderer>,
    gpu_rx: Option<std::sync::mpsc::Receiver<VelloRenderer>>,
    #[cfg(feature = "software")]
    context: Option<Context<Arc<Window>>>,
    #[cfg(feature = "software")]
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    window: Option<Arc<Window>>,
    mouse_pos: (f32, f32),
    mouse_pressed: bool,
    modifiers: ModifiersState,
}

fn to_cursor_icon(name: &str) -> winit::window::CursorIcon {
    match name {
        "pointer" => winit::window::CursorIcon::Pointer,
        "text" => winit::window::CursorIcon::Text,
        "grab" => winit::window::CursorIcon::Grab,
        "grabbing" => winit::window::CursorIcon::Grabbing,
        "move" => winit::window::CursorIcon::Move,
        "not-allowed" => winit::window::CursorIcon::NotAllowed,
        "wait" => winit::window::CursorIcon::Wait,
        "progress" => winit::window::CursorIcon::Progress,
        "crosshair" => winit::window::CursorIcon::Crosshair,
        "zoom-in" => winit::window::CursorIcon::ZoomIn,
        "zoom-out" => winit::window::CursorIcon::ZoomOut,
        "col-resize" => winit::window::CursorIcon::ColResize,
        "row-resize" => winit::window::CursorIcon::RowResize,
        "help" => winit::window::CursorIcon::Help,
        _ => winit::window::CursorIcon::Default,
    }
}

impl RixBrowserApp {
    fn try_ensure_gpu(&mut self) -> Option<&mut VelloRenderer> {
        if self.gpu.is_none() {
            if let Some(ref rx) = self.gpu_rx {
                match rx.try_recv() {
                    Ok(renderer) => {
                        self.gpu = Some(renderer);
                        self.gpu_rx = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        return None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.gpu_rx = None;
                    }
                }
            }
        }
        self.gpu.as_mut()
    }

    #[allow(dead_code)]
    fn ensure_gpu(&mut self) -> &mut VelloRenderer {
        if self.gpu.is_none() {
            if let Some(rx) = self.gpu_rx.take() {
                if let Ok(renderer) = rx.recv() {
                    self.gpu = Some(renderer);
                }
            }
        }
        self.gpu.get_or_insert_with(VelloRenderer::new)
    }
}

impl ApplicationHandler<AppEvent> for RixBrowserApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let (w, h) = self
                .shell
                .as_ref()
                .map(|s| (s.width, s.height))
                .unwrap_or((DEFAULT_WIDTH, DEFAULT_HEIGHT));
            let window_attrs = WindowAttributes::default()
                .with_title("RixBrowser")
                .with_inner_size(LogicalSize::new(w, h));
            let t0 = std::time::Instant::now();
            match event_loop.create_window(window_attrs) {
                Ok(window) => {
                    eprintln!("[rix-timing] create_window took {}ms", t0.elapsed().as_millis());
                    info!("Native desktop window created successfully");
                    println!("Native window active. Press Close or Ctrl+C to exit.");
                    let win = Arc::new(window);
                    let scale = win.scale_factor();
                    let size = win.inner_size();
                    if let Some(ref mut shell) = self.shell {
                        let _ = shell.resize(
                            (size.width as f64 / scale) as f32,
                            (size.height as f64 / scale) as f32,
                        );
                    }
                    #[cfg(feature = "software")]
                    {
                        if let Ok(ctx) = Context::new(win.clone()) {
                            if let Ok(surface) = Surface::new(&ctx, win.clone()) {
                                self.surface = Some(surface);
                            }
                            self.context = Some(ctx);
                        }
                    }
                    self.window = Some(win.clone());
                    event_loop.set_control_flow(ControlFlow::Poll);
                    win.request_redraw();
                }
                Err(e) => {
                    eprintln!("Failed to create OS window: {e}");
                    event_loop.exit();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_none() && self.gpu_rx.is_some() {
            if let Some(ref rx) = self.gpu_rx {
                if let Ok(renderer) = rx.try_recv() {
                    self.gpu = Some(renderer);
                    self.gpu_rx = None;
                    if let Some(win) = self.window.as_ref() {
                        win.request_redraw();
                    }
                }
            }
        }
        if let Some(ref mut shell) = self.shell {
            let _page_tasks = shell.step_event_loop();
            let _navigations = shell.poll_navigation_completions();
            let animation_frame_pending = shell.has_pending_animation_frame_opportunity();
            let needs_redraw = shell.take_needs_redraw();
            if needs_redraw || animation_frame_pending {
                if let Some(ref win) = self.window {
                    win.request_redraw();
                }
            }
            let has_active_work = shell.has_pending_navigations()
                || shell.has_pending_page_layouts();
            if has_active_work || shell.has_active_workers() {
                event_loop.set_control_flow(ControlFlow::WaitUntil(
                    std::time::Instant::now() + std::time::Duration::from_millis(1),
                ));
            } else {
                let idle_bound = std::time::Instant::now() + std::time::Duration::from_millis(32);
                let next_wake = shell
                    .next_event_loop_deadline()
                    .map_or(idle_bound, |deadline| deadline.min(idle_bound));
                event_loop.set_control_flow(ControlFlow::WaitUntil(next_wake));
            }
        } else {
            let idle_bound = std::time::Instant::now() + std::time::Duration::from_millis(32);
            event_loop.set_control_flow(ControlFlow::WaitUntil(idle_bound));
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Control(message) => {
                let redraw = message.requires_redraw();
                if let Some(ref mut shell) = self.shell {
                    pollster::block_on(shell.handle_control_message(message));
                }
                if redraw {
                    if let Some(ref win) = self.window {
                        win.request_redraw();
                    }
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                info!("Window close requested, exiting.");
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(ref mut shell) = self.shell {
                    let scale = self
                        .window
                        .as_ref()
                        .map(|win| win.scale_factor())
                        .unwrap_or(1.0);
                    let _ = shell.resize(
                        (size.width as f64 / scale) as f32,
                        (size.height as f64 / scale) as f32,
                    );
                }
                if let Some(gpu) = self.try_ensure_gpu() {
                    gpu.resize(size.width.max(1), size.height.max(1));
                }
                #[cfg(feature = "software")]
                if let Some(ref mut surface) = self.surface {
                    if let (Some(w), Some(h)) = (
                        NonZeroU32::new(size.width.max(1)),
                        NonZeroU32::new(size.height.max(1)),
                    ) {
                        let _ = surface.resize(w, h);
                    }
                }
                if let Some(ref win) = self.window {
                    win.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self
                    .window
                    .as_ref()
                    .map(|win| win.scale_factor())
                    .unwrap_or(1.0);
                self.mouse_pos = ((position.x / scale) as f32, (position.y / scale) as f32);
                if let Some(ref mut shell) = self.shell {
                    let cursor_name = shell.cursor_at(self.mouse_pos.0, self.mouse_pos.1);
                    if let Some(ref win) = self.window {
                        win.set_cursor(to_cursor_icon(cursor_name));
                    }
                    if shell.update_chrome_pointer(Some(self.mouse_pos), self.mouse_pressed) {
                        if let Some(ref win) = self.window {
                            win.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.mouse_pressed = false;
                if let Some(ref win) = self.window {
                    win.set_cursor(winit::window::CursorIcon::Default);
                }
                if let Some(ref mut shell) = self.shell {
                    shell.update_chrome_pointer(None, false);
                }
                if let Some(ref win) = self.window {
                    win.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let (Some(ref mut shell), Some(ref win)) = (&mut self.shell, &self.window) {
                    let size = win.inner_size();
                    let _ = shell.resize(
                        (size.width as f64 / scale_factor) as f32,
                        (size.height as f64 / scale_factor) as f32,
                    );
                    win.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button,
                ..
            } => {
                let btn_num = match button {
                    MouseButton::Left => 0,
                    MouseButton::Middle => 1,
                    MouseButton::Right => 2,
                    _ => 0,
                };
                if btn_num == 0 {
                    self.mouse_pressed = false;
                }
                if let Some(ref mut shell) = self.shell {
                    shell.update_chrome_pointer(Some(self.mouse_pos), false);
                    let _ = pollster::block_on(shell.handle_input(IpcInputEvent::MouseUp {
                        x: self.mouse_pos.0,
                        y: self.mouse_pos.1,
                        button: btn_num,
                    }));
                }
                if let Some(ref win) = self.window {
                    win.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                let btn_num = match button {
                    MouseButton::Left => 0,
                    MouseButton::Middle => 1,
                    MouseButton::Right => 2,
                    _ => 0,
                };
                if btn_num == 0 {
                    self.mouse_pressed = true;
                }
                tracing::info!(x = self.mouse_pos.0, y = self.mouse_pos.1, button = btn_num, "Mouse press");
                if let Some(ref mut shell) = self.shell {
                    shell.update_chrome_pointer(Some(self.mouse_pos), true);
                    let _ = pollster::block_on(shell.handle_input(IpcInputEvent::MouseDown {
                        x: self.mouse_pos.0,
                        y: self.mouse_pos.1,
                        button: btn_num,
                    }));
                    if let Some(ref win) = self.window {
                        win.request_redraw();
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll_amount = match delta {
                    MouseScrollDelta::LineDelta(_x, y) => -y * 40.0,
                    MouseScrollDelta::PixelDelta(pos) => {
                        let scale = self
                            .window
                            .as_ref()
                            .map(|win| win.scale_factor())
                            .unwrap_or(1.0);
                        (-pos.y / scale) as f32
                    }
                };
                if let Some(ref mut shell) = self.shell {
                    shell.scroll_active_tab(scroll_amount);
                    if let Some(ref win) = self.window {
                        win.request_redraw();
                    }
                }
            }
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                if key_event.state == ElementState::Pressed {
                    if let Some(ref mut shell) = self.shell {
                        let key = match &key_event.logical_key {
                            Key::Named(NamedKey::Enter) => "Enter".to_string(),
                            Key::Named(NamedKey::Backspace) => "Backspace".to_string(),
                            Key::Named(NamedKey::Delete) => "Delete".to_string(),
                            Key::Named(NamedKey::Insert) => "Insert".to_string(),
                            Key::Named(NamedKey::Escape) => "Escape".to_string(),
                            Key::Named(NamedKey::ArrowLeft) => "ArrowLeft".to_string(),
                            Key::Named(NamedKey::ArrowRight) => "ArrowRight".to_string(),
                            Key::Named(NamedKey::ArrowUp) => "ArrowUp".to_string(),
                            Key::Named(NamedKey::ArrowDown) => "ArrowDown".to_string(),
                            Key::Named(NamedKey::Tab) => "Tab".to_string(),
                            Key::Named(NamedKey::Home) => "Home".to_string(),
                            Key::Named(NamedKey::End) => "End".to_string(),
                            Key::Named(NamedKey::Space) => " ".to_string(),
                            Key::Named(NamedKey::PageUp) => "PageUp".to_string(),
                            Key::Named(NamedKey::PageDown) => "PageDown".to_string(),
                            Key::Named(NamedKey::F1) => "F1".to_string(),
                            Key::Named(NamedKey::F2) => "F2".to_string(),
                            Key::Named(NamedKey::F3) => "F3".to_string(),
                            Key::Named(NamedKey::F4) => "F4".to_string(),
                            Key::Named(NamedKey::F5) => "F5".to_string(),
                            Key::Named(NamedKey::F6) => "F6".to_string(),
                            Key::Named(NamedKey::F7) => "F7".to_string(),
                            Key::Named(NamedKey::F8) => "F8".to_string(),
                            Key::Named(NamedKey::F9) => "F9".to_string(),
                            Key::Named(NamedKey::F10) => "F10".to_string(),
                            Key::Named(NamedKey::F11) => "F11".to_string(),
                            Key::Named(NamedKey::F12) => "F12".to_string(),
                            Key::Character(text) => text.to_string(),
                            _ => key_event.text.as_deref().unwrap_or("").to_string(),
                        };
                        let code = match key_event.physical_key {
                            winit::keyboard::PhysicalKey::Code(code) => format!("{code:?}"),
                            winit::keyboard::PhysicalKey::Unidentified(_) => String::new(),
                        };
                        let modifiers = self.modifiers;
                        pollster::block_on(shell.handle_native_key(
                            &key,
                            &code,
                            modifiers.control_key(),
                            modifiers.alt_key(),
                            modifiers.shift_key(),
                            modifiers.super_key(),
                        ));
                        if let Some(ref win) = self.window {
                            win.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let win = self.window.clone();
                if let (Some(ref mut shell), Some(win)) = (&mut self.shell, win) {
                    let _ = shell.animation_frame_opportunity(std::time::Instant::now());
                    let size = win.inner_size();
                    let width = size.width.max(1);
                    let height = size.height.max(1);

                    let (logical_scene, stats) = shell.render_frame();
                    let mut scene = vello::Scene::new();
                    scene.append(
                        &logical_scene,
                        Some(vello::kurbo::Affine::scale(win.scale_factor())),
                    );
                    tracing::debug!(
                        "GPU frame {}: {:.2}ms ({:.0} FPS, 120fps={})",
                        stats.frame_number,
                        stats.frame_duration_ms,
                        stats.fps,
                        stats.is_120_fps_capable
                    );

                    if let Some(gpu) = self.try_ensure_gpu() {
                        let surface_ready =
                            pollster::block_on(gpu.ensure_surface(win.clone(), width, height));
                        match surface_ready {
                            Ok(()) => {
                                win.pre_present_notify();
                                if let Err(e) = gpu.render(&scene) {
                                    eprintln!("GPU present error: {e}");
                                    #[cfg(feature = "software")]
                                    self.present_software_fallback(&scene, width, height);
                                    #[cfg(not(feature = "software"))]
                                    {
                                        let _ = &scene;
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("GPU surface error: {e}");
                                #[cfg(feature = "software")]
                                self.present_software_fallback(&scene, width, height);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(feature = "software")]
impl RixBrowserApp {
    fn present_software_fallback(&mut self, scene: &vello::Scene, width: u32, height: u32) {
        let gpu = self.ensure_gpu();
        let rgba = match pollster::block_on(gpu.render_to_rgba(scene, width, height)) {
            Ok(rgba) => rgba,
            Err(error) => {
                eprintln!("Offscreen render error: {error}");
                return;
            }
        };
        if let Some(ref mut surface) = self.surface {
            if let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
                let _ = surface.resize(w, h);
            }
            if let Ok(mut buffer) = surface.buffer_mut() {
                for (pixel, channels) in buffer.iter_mut().zip(rgba.chunks_exact(4)) {
                    *pixel = (u32::from(channels[0]) << 16)
                        | (u32::from(channels[1]) << 8)
                        | u32::from(channels[2]);
                }
                if let Err(error) = buffer.present() {
                    eprintln!("Softbuffer present error: {error}");
                }
            }
        }
    }
}
