
#![deny(unsafe_code)]

use crate::customization::{CustomizationConfig, CustomizationManager, TabOrientation, ThemeMode};
use crate::resource_policy::{NavigationResourcePolicy, SourceBudget};
use crate::surface::WebSurface;
use crate::text_editing::{edit_text, KeyboardInput, TextSelection};
use rix_core::url::RixUrl;
use rix_css::computed::{compute_tree_styles, LayoutContext};
use rix_css::rule_index::RuleIndex;
use rix_css::stylesheet::StyleSheet;
use rix_dom::node::NodeData;
use rix_dom::tree::Document;
use rix_dom::NodeId;
use rix_html::parse_html;
use rix_ipc::process::BrowserProcessManager;
use rix_ipc::protocol::{IpcInputEvent, IpcMessage};
use rix_js::dom_bridge::{dispatch_dom_event_to_js, register_dom_bridge, DomBridgeState};
use rix_js::privileged::{register_privileged_api_if_eligible, PrivilegedHostState};
use rix_js::runtime::{JsRuntime, JsRuntimeConfig};
use rix_js::web_api::{
    register_web_apis, EventLoop, ResourceLoadKind, ResourceLoadRecord, WebApiState,
};
use rix_layout::taffy_adapter::TaffyLayoutEngine;
use rix_layout::tree::LayoutTree;
use rix_net::client::HttpClient;
use rix_render::compositor::{Compositor, FrameStats};
use rix_render::display_list::{DisplayList, DrawCommand};
use rix_render::image_resource::ImageResource;
use rix_text::font_loader::FontDatabase;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

mod chrome_input;
mod control;
mod layout_sync;
mod loader;
mod loading_page;
mod navigation;
mod page_input;
mod page_layout;
mod preload_scanner;
mod render;
mod workers;

use crate::page_style::{
    compute_page_layout, document_base_url, resolve_external_resource, resolve_page_link,
};
use loader::*;
use loading_page::*;
use page_layout::*;
static NEXT_PAGE_DISPLAY_REVISION: AtomicU64 = AtomicU64::new(1);
static NEXT_NAVIGATION_ID: AtomicU64 = AtomicU64::new(1);
const MAX_RESOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_DOCUMENT_NODES: usize = 100_000;
const MAX_DOCUMENT_DEPTH: usize = 512;
const MAX_PAGE_RESOURCE_TRACE: usize = 1024;

#[derive(Default, PartialEq)]
struct ChromeFontResources {
    faces: Vec<(u64, u32)>,
    fallbacks: [Option<rix_text::FontId>; 4],
}

impl ChromeFontResources {
    fn capture(fonts: &FontDatabase) -> Self {
        Self {
            faces: (0..fonts.len())
                .filter_map(|id| fonts.get_font(rix_text::FontId(id)))
                .map(|font| (font.blob().id(), font.face_index))
                .collect(),
            fallbacks: [
                fonts.default_sans_serif,
                fonts.default_serif,
                fonts.default_monospace,
                fonts.default_emoji,
            ],
        }
    }
}

fn next_page_display_revision() -> u64 {
    NEXT_PAGE_DISPLAY_REVISION.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone)]
struct NavigationRequest {
    id: u64,
    tab_id: u32,
    url: String,
    previous_url: String,
    previous_back: Vec<String>,
    previous_forward: Vec<String>,
    history_override: Option<(Vec<String>, Vec<String>)>,
}

struct NavigationCompletion {
    request: NavigationRequest,
    content: NavigationContent,
    is_final: bool,
}

struct PageLayoutCompletion {
    tab_id: u32,
    revision: u64,
    request_id: u64,
    html: String,
    document: Document,
    layout: LayoutTree,
    layout_viewport: (f32, f32),
    worker_snapshot: Option<InstalledWorkerSnapshot>,
}

struct InstalledWorkerSnapshot {
    state: crate::page_worker::SnapshotState,
    nodes: HashMap<crate::page_worker::NodeToken, NodeId>,
}

struct DeferredWorkerSnapshot {
    generation: u64,
    html: String,
    state: crate::page_worker::SnapshotState,
}

enum PageLayoutResult {
    Ready(PageLayoutCompletion),
    Failed { tab_id: u32, request_id: u64 },
}

struct NavigationContent {
    html: String,
    document: Document,
    prepared_layout: Option<LayoutTree>,
    layout_viewport: Option<(u32, u32)>,
    final_url: String,
    status_message: String,
    stylesheets: HashMap<String, String>,
    scripts: Vec<crate::page_worker::PageScript>,
    failed_resources: usize,
    navigation_clock: Option<(rix_net::NavigationTiming, std::time::Instant)>,
    resource_trace: Vec<ResourceLoadRecord>,
}

struct PageImageCompletion {
    tab_id: u32,
    generation: u64,
    url: String,
    image: Result<ImageResource, String>,
    resource_record: ResourceLoadRecord,
}

struct DynamicScriptCompletion {
    tab_id: u32,
    generation: u64,
    target: crate::page_worker::NodeToken,
    source_url: String,
    source: Result<String, String>,
    resource_record: ResourceLoadRecord,
}

struct ModuleLoadCompletion {
    tab_id: u32,
    generation: u64,
    request_id: u64,
    request_url: String,
    result: Result<(String, String), String>,
    resource_record: ResourceLoadRecord,
}

const MAX_PAGE_IMAGE_BYTES: usize = 128 * 1024 * 1024;
const MAX_DECODED_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

pub(super) fn retain_resource_record(page: &mut PageContent, record: ResourceLoadRecord) {
    if page.resource_trace.len() < MAX_PAGE_RESOURCE_TRACE {
        page.resource_trace.push(record);
    } else {
        page.resource_trace_truncated = true;
    }
}

enum ScriptSource {
    Inline {
        source: String,
        element_index: usize,
        module: bool,
    },
    External {
        key: String,
        element_index: usize,
        module: bool,
    },
}

#[derive(Clone, Copy)]
enum ExternalResourceKind {
    Stylesheet,
    Script,
    Module,
}

#[derive(Error, Debug)]
pub enum ShellError {
    #[error("Layout Computation Error: {0}")]
    Layout(String),

    #[error("JavaScript Error: {0}")]
    Js(String),

    #[error("IPC Error: {0}")]
    Ipc(#[from] rix_ipc::protocol::IpcError),
}

impl From<String> for ShellError {
    fn from(err: String) -> Self {
        Self::Layout(err)
    }
}

pub struct PageRuntimeContext {
    pub runtime: JsRuntime,
    pub web_state: Rc<RefCell<WebApiState>>,
    pub dom_state: Rc<RefCell<DomBridgeState>>,
    pub event_loop: EventLoop,
}

#[derive(Clone)]
pub struct PageContent {
    pub url: String,
    pub title: String,
    pub html: String,
    pub external_stylesheets: HashMap<String, String>,
    pub document: Document,
    pub layout_tree: LayoutTree,
    layout_viewport: (f32, f32),
    pub display_list: DisplayList,
    pub images: HashMap<String, ImageResource>,
    image_attempts: std::collections::HashSet<String>,
    dynamic_script_attempts: std::collections::HashSet<crate::page_worker::NodeToken>,
    module_source_cache: HashMap<String, (String, String)>,
    dynamic_script_semaphore: std::sync::Arc<tokio::sync::Semaphore>,
    loaded_source_bytes: usize,
    image_generation: u64,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub is_loading: bool,
    pub status_message: String,
    pub in_page_input: String,
    pub input_selection: TextSelection,
    pub in_page_focused: bool,
    pub focused_input: Option<rix_dom::NodeId>,
    pub back_stack: Vec<String>,
    pub forward_stack: Vec<String>,
    pub runtime_context: Option<Rc<RefCell<PageRuntimeContext>>>,
    pub page_worker: Option<Rc<RefCell<crate::page_worker::PageWorker>>>,
    pub resource_trace: Vec<ResourceLoadRecord>,
    pub resource_trace_truncated: bool,
    pub rendered_dom_revision: u64,
    pub display_revision: u64,
    pub node_count: usize,
    pub scheduled_images_revision: u64,
    pub scheduled_images_scroll_y: f32,
    worker_layout_sync: WorkerLayoutSync,
    worker_animation_frame_requested: bool,
    worker_animation_frame_in_flight: bool,
    worker_animation_frame_ready_to_paint: bool,
    pub pending_history_traversal: bool,
}

#[derive(Clone, Default)]
struct WorkerLayoutSync {
    viewport: Option<(f32, f32)>,
    scroll_x: Option<f32>,
    scroll_y: Option<f32>,
    images: std::collections::HashSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClickAction {
    SwitchTab(u32),
    CloseTab(u32),
    NewTab,
    ToggleTheme,
    ToggleTabOrientation,
    NavBack,
    NavForward,
    NavReload,
    FocusOmnibox,
    UnfocusOmnibox,
    NavigateUrl(String),
    FocusInPageInput,
}

pub struct BrowserShell {
    pub width: f32,
    pub height: f32,
    pub document: Rc<RefCell<Document>>,
    pub chrome_stylesheet: StyleSheet,
    pub customization: CustomizationManager,
    pub runtime: JsRuntime,
    pub web_state: Rc<RefCell<WebApiState>>,
    pub dom_state: Rc<RefCell<DomBridgeState>>,
    pub priv_state: Rc<RefCell<PrivilegedHostState>>,
    pub process_mgr: BrowserProcessManager,
    pub web_surface: WebSurface,
    pub layout_tree: Option<LayoutTree>,
    pub font_database: FontDatabase,
    pub compositor: Compositor,
    pub chrome_layer_id: usize,
    pub surface_layer_id: usize,
    pub overlay_layer_id: usize,
    pub internal_scroll_y: f32,
    pub pages: HashMap<u32, PageContent>,
    pub storage: Option<rix_storage::StorageEngine>,
    pub http_client: HttpClient,
    pub navigation_resource_policy: NavigationResourcePolicy,
    pub page_worker_command: Option<crate::page_worker::WorkerCommand>,
    pub homepage: String,
    pub omnibox_text: String,
    pub omnibox_focused: bool,
    omnibox_selection: TextSelection,
    pub search_engine_template: String,
    chrome_dom_revision: u64,
    chrome_style_revision: u64,
    chrome_layout_viewport: (f32, f32),
    chrome_layout_engine: TaffyLayoutEngine,
    chrome_font_resources: ChromeFontResources,
    chrome_ua_index: RuleIndex,
    chrome_author_index: RuleIndex,
    surface_layer_key: Option<(u32, u64, bool, u32, u32, u32, u32, u32, u32)>,
    navigation_tx: tokio::sync::mpsc::UnboundedSender<NavigationCompletion>,
    navigation_rx: tokio::sync::mpsc::UnboundedReceiver<NavigationCompletion>,
    pending_navigations: HashMap<u32, u64>,
    navigation_tasks: HashMap<u32, tokio::task::JoinHandle<()>>,
    page_layout_tx: tokio::sync::mpsc::UnboundedSender<PageLayoutResult>,
    page_layout_rx: tokio::sync::mpsc::UnboundedReceiver<PageLayoutResult>,
    pending_page_layouts: HashMap<u32, u64>,
    layout_tokens: HashMap<u32, std::sync::Arc<std::sync::atomic::AtomicU64>>,
    deferred_worker_snapshots: HashMap<u32, DeferredWorkerSnapshot>,
    event_loop: EventLoop,
    image_tx: tokio::sync::mpsc::UnboundedSender<PageImageCompletion>,
    image_rx: tokio::sync::mpsc::UnboundedReceiver<PageImageCompletion>,
    image_tasks: HashMap<u32, Vec<tokio::task::JoinHandle<()>>>,
    pending_images: std::collections::HashSet<(u32, u64, String)>,
    dynamic_script_tx: tokio::sync::mpsc::UnboundedSender<DynamicScriptCompletion>,
    dynamic_script_rx: tokio::sync::mpsc::UnboundedReceiver<DynamicScriptCompletion>,
    dynamic_script_tasks: HashMap<u32, Vec<tokio::task::JoinHandle<()>>>,
    pending_dynamic_scripts: std::collections::HashSet<(u32, u64, crate::page_worker::NodeToken)>,
    module_load_tx: tokio::sync::mpsc::UnboundedSender<ModuleLoadCompletion>,
    module_load_rx: tokio::sync::mpsc::UnboundedReceiver<ModuleLoadCompletion>,
    module_load_tasks: HashMap<u32, Vec<tokio::task::JoinHandle<()>>>,
    pending_module_loads: std::collections::HashSet<(u32, u64, u64)>,
    worker_defaults: HashMap<(u32, u64), ClickAction>,
    worker_keys: HashMap<(u32, u64), PendingPageKey>,
    pub media_engines: HashMap<(u32, rix_ipc::media::MediaSessionId), rix_media::PlaybackEngine>,
    pub needs_redraw: bool,
}

struct PendingPageKey {
    input: KeyboardInput,
    target: Option<crate::page_worker::NodeToken>,
    focus_epoch: u64,
    generation: u64,
}

impl BrowserShell {
    pub fn new(width: f32, height: f32) -> Result<Self, ShellError> {
        Self::new_with_customization(
            width,
            height,
            CustomizationManager::new(CustomizationConfig::default()),
        )
    }

    pub fn take_needs_redraw(&mut self) -> bool {
        std::mem::replace(&mut self.needs_redraw, false)
    }

    pub fn set_needs_redraw(&mut self) {
        self.needs_redraw = true;
    }

    pub fn new_with_customization(
        width: f32,
        height: f32,
        customization: CustomizationManager,
    ) -> Result<Self, ShellError> {
        let chrome_url = RixUrl::parse("rix://chrome/browser.html").unwrap();
        let browser_html = include_str!("../../assets/browser.html");
        let browser_css = include_str!("../../assets/browser.css");
        let browser_js = include_str!("../../assets/browser.js");

        let doc = parse_html(browser_html);
        static BROWSER_STYLESHEET: std::sync::OnceLock<StyleSheet> = std::sync::OnceLock::new();
        let chrome_stylesheet = BROWSER_STYLESHEET
            .get_or_init(|| (*StyleSheet::parse(browser_css)).clone())
            .clone();
        let chrome_author_index = RuleIndex::from_stylesheet(&chrome_stylesheet);

        let mut config = JsRuntimeConfig::default();
        config.origin = Some(chrome_url.as_str().to_string());
        let mut runtime = JsRuntime::new(config);

        let web_state = Rc::new(RefCell::new(WebApiState::new(Some(chrome_url.clone()))));
        register_web_apis(runtime.context_mut(), web_state.clone())
            .map_err(|e| ShellError::Js(format!("{}", e)))?;

        let dom_state = Rc::new(RefCell::new(DomBridgeState::new(doc)));
        register_dom_bridge(runtime.context_mut(), dom_state.clone())
            .map_err(|e| ShellError::Js(format!("{}", e)))?;

        let priv_state = Rc::new(RefCell::new(PrivilegedHostState::new(Some(chrome_url))));
        register_privileged_api_if_eligible(runtime.context_mut(), priv_state.clone())
            .map_err(|e| ShellError::Js(format!("{}", e)))?;

        runtime
            .evaluate(browser_js)
            .map_err(|e| ShellError::Js(format!("{}", e)))?;

        let mut font_database = FontDatabase::new();
        font_database.scan_system_fonts();

        let mut compositor = Compositor::new();
        let chrome_layer_id = compositor.create_layer(10);
        let surface_layer_id = compositor.create_layer(20);
        let overlay_layer_id = compositor.create_layer(30);

        let event_loop = EventLoop::new(web_state.clone());
        let process_mgr = BrowserProcessManager::new();
        let (navigation_tx, navigation_rx) = tokio::sync::mpsc::unbounded_channel();
        let (page_layout_tx, page_layout_rx) = tokio::sync::mpsc::unbounded_channel();

        let doc_rc = dom_state.borrow().document.clone();
        let storage = rix_storage::StorageEngine::in_memory().ok();
        let http_client =
            HttpClient::new(storage.clone()).map_err(|e| ShellError::Layout(format!("{e}")))?;
        rix_js::web_api::set_shared_network_client(http_client.clone());

        let chrome_ua_index = RuleIndex::default_ua();

        let (image_tx, image_rx) = tokio::sync::mpsc::unbounded_channel();
        let (dynamic_script_tx, dynamic_script_rx) = tokio::sync::mpsc::unbounded_channel();
        let (module_load_tx, module_load_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut shell = Self {
            width,
            height,
            document: doc_rc,
            chrome_stylesheet,
            customization,
            runtime,
            web_state,
            dom_state,
            priv_state,
            process_mgr,
            web_surface: WebSurface::default(),
            layout_tree: None,
            font_database,
            compositor,
            chrome_layer_id,
            surface_layer_id,
            overlay_layer_id,
            internal_scroll_y: 0.0,
            pages: HashMap::new(),
            storage,
            http_client,
            navigation_resource_policy: NavigationResourcePolicy::default(),
            page_worker_command: None,
            homepage: "rix://newtab".to_string(),
            omnibox_text: "".to_string(),
            omnibox_focused: false,
            omnibox_selection: TextSelection::default(),
            search_engine_template: "https://www.google.com/search?q={query}".to_string(),
            chrome_dom_revision: 0,
            chrome_style_revision: 0,
            chrome_layout_viewport: (0.0, 0.0),
            chrome_layout_engine: TaffyLayoutEngine::new(),
            chrome_font_resources: ChromeFontResources::default(),
            chrome_ua_index,
            chrome_author_index,
            surface_layer_key: None,
            navigation_tx,
            navigation_rx,
            pending_navigations: HashMap::new(),
            navigation_tasks: HashMap::new(),
            page_layout_tx,
            page_layout_rx,
            pending_page_layouts: HashMap::new(),
            layout_tokens: HashMap::new(),
            deferred_worker_snapshots: HashMap::new(),
            event_loop,
            image_tx,
            image_rx,
            image_tasks: HashMap::new(),
            pending_images: std::collections::HashSet::new(),
            dynamic_script_tx,
            dynamic_script_rx,
            dynamic_script_tasks: HashMap::new(),
            pending_dynamic_scripts: std::collections::HashSet::new(),
            module_load_tx,
            module_load_rx,
            module_load_tasks: HashMap::new(),
            pending_module_loads: std::collections::HashSet::new(),
            worker_defaults: HashMap::new(),
            worker_keys: HashMap::new(),
            media_engines: HashMap::new(),
            needs_redraw: true,
        };

        shell.apply_customization_to_chrome()?;
        let initial_url = "about:blank";
        let initial_page = shell.build_page_from_html(
            initial_url,
            &loading_page_html(initial_url, &shell.shell_theme()),
        );
        shell.omnibox_text = initial_page.url.clone();
        {
            let mut host = shell.priv_state.borrow_mut();
            let active_tab_id = host.active_tab_id;
            if let Some(tab) = host.tabs.iter_mut().find(|tab| tab.id == active_tab_id) {
                tab.url = initial_page.url.clone();
                tab.title = "New Tab".to_string();
            }
        }
        shell.pages.insert(1, initial_page);
        shell.omnibox_selection = TextSelection::collapsed(shell.omnibox_text.chars().count());
        shell.set_chrome_omnibox_value(&shell.omnibox_text.clone());
        shell.notify_chrome_tabs_changed();
        let _ = shell.recompute_layout();
        Ok(shell)
    }

    pub fn set_storage(&mut self, storage: rix_storage::StorageEngine) {
        self.http_client.set_storage(Some(storage.clone()));
        rix_js::web_api::set_shared_network_client(self.http_client.clone());
        self.storage = Some(storage);
    }
}

impl Drop for BrowserShell {
    fn drop(&mut self) {
        for (_, task) in self.navigation_tasks.drain() {
            task.abort();
        }
    }
}
