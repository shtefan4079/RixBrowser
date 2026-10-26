
use crate::page_style::{
    compute_page_style_for_node, compute_page_styles_and_layout_with_environment,
    document_base_url, resolve_external_resource, StyleEnvironment,
};
use rix_css::ComputedStyle;
use rix_dom::node::NodeData;
use rix_dom::tree::Document;
use rix_dom::NodeId;
use rix_js::dom_bridge::{LayoutHost, LayoutSnapshot};
use rix_layout::taffy_adapter::TaffyLayoutEngine;
use rix_text::font_loader::FontDatabase;
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

static SYSTEM_FONTS_CACHE: OnceLock<FontDatabase> = OnceLock::new();

pub fn cached_system_fonts() -> &'static FontDatabase {
    SYSTEM_FONTS_CACHE.get_or_init(|| {
        let mut fonts = FontDatabase::new();
        fonts.scan_system_fonts();
        fonts
    })
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct LayoutInputs {
    pub stylesheets: HashMap<String, String>,
    pub viewport: (f32, f32),
    pub scroll: (f32, f32),
    pub image_sizes: HashMap<String, (f32, f32)>,
}

struct CachedLayout {
    key: (u64, u64, u64),
    snapshot: Rc<LayoutSnapshot>,
}

struct CachedNodeStyles {
    key: (u64, u64),
    styles: Rc<HashMap<NodeId, ComputedStyle>>,
}

pub struct PageLayoutHost {
    url: String,
    inputs: RefCell<LayoutInputs>,
    inputs_revision: RefCell<u64>,
    scroll_revision: Cell<u64>,
    fonts: OnceCell<FontDatabase>,
    cache: RefCell<Option<CachedLayout>>,
    full_page_styles: RefCell<Option<CachedNodeStyles>>,
    node_styles: RefCell<Option<CachedNodeStyles>>,
    style_environment: RefCell<Option<Rc<StyleEnvironment>>>,
    pending_scroll_request: Cell<Option<(f32, f32)>>,
    pending_scroll_event: Cell<bool>,
}

impl PageLayoutHost {
    pub fn new(url: impl Into<String>, inputs: LayoutInputs) -> Self {
        Self {
            url: url.into(),
            inputs: RefCell::new(inputs),
            inputs_revision: RefCell::new(0),
            scroll_revision: Cell::new(0),
            fonts: OnceCell::new(),
            cache: RefCell::new(None),
            full_page_styles: RefCell::new(None),
            node_styles: RefCell::new(None),
            style_environment: RefCell::new(None),
            pending_scroll_request: Cell::new(None),
            pending_scroll_event: Cell::new(false),
        }
    }

    pub fn with_fonts(url: impl Into<String>, inputs: LayoutInputs, fonts: FontDatabase) -> Self {
        let host = Self::new(url, inputs);
        let _ = host.fonts.set(fonts);
        host
    }

    pub fn update(&self, change: impl FnOnce(&mut LayoutInputs)) {
        change(&mut self.inputs.borrow_mut());
        *self.inputs_revision.borrow_mut() += 1;
    }

    pub fn set_scroll(&self, x: f32, y: f32) {
        let mut inputs = self.inputs.borrow_mut();
        if inputs.scroll != (x, y) {
            inputs.scroll = (x, y);
            self.scroll_revision
                .set(self.scroll_revision.get().wrapping_add(1));
            self.pending_scroll_event.set(true);
        }
    }

    pub fn has_pending_scroll_event(&self) -> bool {
        self.pending_scroll_event.get()
    }

    pub fn take_scroll_event(&self) -> bool {
        self.pending_scroll_event.replace(false)
    }

    pub fn take_scroll_request(&self) -> Option<(f32, f32)> {
        self.pending_scroll_request.take()
    }

    fn fonts(&self) -> &FontDatabase {
        self.fonts.get_or_init(|| cached_system_fonts().clone())
    }

    fn style_environment(&self, document: &Document) -> Rc<StyleEnvironment> {
        let inputs = self.inputs.borrow();
        let (width, height) = inputs.viewport;
        let mut cached = self.style_environment.borrow_mut();
        if let Some(current) = cached.as_ref() {
            if current.matches(document, &self.url, &inputs.stylesheets, width, height) {
                return Rc::clone(current);
            }
        }
        let built = Rc::new(StyleEnvironment::build(
            document,
            &self.url,
            &inputs.stylesheets,
            width,
            height,
        ));
        *cached = Some(Rc::clone(&built));
        built
    }

    fn compute(&self, document: &Document) -> LayoutSnapshot {
        let t_all = std::time::Instant::now();
        let t_inputs = std::time::Instant::now();
        let inputs = self.inputs.borrow();
        let (width, height) = inputs.viewport;
        let d_inputs = t_inputs.elapsed().as_millis();
        let t_fonts = std::time::Instant::now();
        let fonts = self.fonts();
        let d_fonts = t_fonts.elapsed().as_millis();
        let t_key = std::time::Instant::now();
        let node_key = (
            document.style_revision(),
            *self.inputs_revision.borrow(),
        );
        let cached_styles = {
            let guard = self.full_page_styles.borrow();
            if let Some(c) = guard.as_ref() {
                if c.key == node_key && !c.styles.is_empty() {
                    Some(Rc::clone(&c.styles))
                } else {
                    None
                }
            } else {
                None
            }
        };
        let d_key = t_key.elapsed().as_millis();
        let t_branch = std::time::Instant::now();
        let (mut tree, styles) = if let Some(styles) = cached_styles {
            let t_dom = std::time::Instant::now();
            let mut layout = rix_layout::tree::LayoutTree::from_dom(document, document.root(), &styles);
            let from_dom_ms = t_dom.elapsed().as_millis();
            let t_taffy = std::time::Instant::now();
            let mut engine = TaffyLayoutEngine::new();
            if let Err(error) = engine.compute_layout_with_fonts(
                &mut layout,
                width.max(1.0),
                height.max(1.0),
                fonts,
            ) {
                tracing::warn!("Page layout failed: {error}");
            }
            let taffy_ms = t_taffy.elapsed().as_millis();
            if std::env::var_os("RIX_WORKER_TRACE").is_some() && (from_dom_ms > 20 || taffy_ms > 20) {
                eprintln!("[rix-cached-layout] from_dom={}ms taffy={}ms", from_dom_ms, taffy_ms);
            }
            (layout, styles)
        } else {
            let t_env = std::time::Instant::now();
            let environment = self.style_environment(document);
            let env_ms = t_env.elapsed().as_millis();
            let t_calc = std::time::Instant::now();
            let (tree, styles) =
                compute_page_styles_and_layout_with_environment(document, &environment, fonts);
            let calc_ms = t_calc.elapsed().as_millis();
            if std::env::var_os("RIX_WORKER_TRACE").is_some() && (env_ms > 20 || calc_ms > 20) {
                eprintln!("[rix-compute-sub] env={}ms calc={}ms", env_ms, calc_ms);
            }
            let styles = Rc::new(styles);
            *self.full_page_styles.borrow_mut() = Some(CachedNodeStyles {
                key: node_key,
                styles: Rc::clone(&styles),
            });
            (tree, styles)
        };
        let d_branch = t_branch.elapsed().as_millis();
        let t_img = std::time::Instant::now();
        if !inputs.image_sizes.is_empty() {
            let base = document_base_url(document, &self.url);
            let mut changed = false;
            for node in document.descendants(document.root()) {
                let Some(NodeData::Element(element)) = document.get(node) else {
                    continue;
                };
                if element.name.local.as_str() != "img" {
                    continue;
                }
                let size = element
                    .get_attribute("src")
                    .and_then(|source| image_key(&base, source))
                    .and_then(|url| inputs.image_sizes.get(&url));
                if let Some((image_width, image_height)) = size {
                    changed |= tree.set_image_intrinsic_size(node, *image_width, *image_height);
                }
            }
            if changed {
                if let Err(error) = TaffyLayoutEngine::new().compute_layout_with_fonts(
                    &mut tree,
                    width.max(1.0),
                    height.max(1.0),
                    fonts,
                ) {
                    tracing::warn!("Image relayout failed: {error}");
                }
            }
        }
        let d_img = t_img.elapsed().as_millis();
        let t_snap = std::time::Instant::now();
        let snap = LayoutSnapshot::with_styles(tree, styles);
        let d_snap = t_snap.elapsed().as_millis();
        let total = t_all.elapsed().as_millis();
        if std::env::var_os("RIX_WORKER_TRACE").is_some() && total > 50 {
            eprintln!(
                "[rix-compute-breakdown] total={}ms inputs={}ms fonts={}ms key={}ms branch={}ms img={}ms snap={}ms",
                total, d_inputs, d_fonts, d_key, d_branch, d_img, d_snap
            );
        }
        snap
    }
}

fn composed_parent(document: &Document, node: NodeId) -> Option<NodeId> {
    let parent = document.parent(node)?;
    match document.get(parent) {
        Some(NodeData::ShadowRoot { host, .. }) => Some(*host),
        _ => Some(parent),
    }
}

fn image_key(base: &str, source: &str) -> Option<String> {
    let source = source.trim();
    if source.starts_with("data:image/") {
        Some(source.to_string())
    } else {
        resolve_external_resource(base, source).map(|(url, _)| url)
    }
}

impl LayoutHost for PageLayoutHost {
    fn layout(&self, document: &Document) -> Option<Rc<LayoutSnapshot>> {
        let key = (
            document.revision(),
            document.style_revision(),
            *self.inputs_revision.borrow(),
        );
        if let Some(cached) = self.cache.borrow().as_ref() {
            if cached.key == key {
                return Some(cached.snapshot.clone());
            }
        }
        let compute_started = std::time::Instant::now();
        let snapshot = Rc::new(self.compute(document));
        let compute_ms = compute_started.elapsed().as_millis();
        if std::env::var_os("RIX_WORKER_TRACE").is_some() {
            let t_count = std::time::Instant::now();
            let nodes = document.descendants(document.root()).count();
            let count_ms = t_count.elapsed().as_millis();
            eprintln!(
                "[rix-layout] compute {}ms (self.compute={}ms count={}ms) nodes={nodes} revision={} style_revision={} inputs={} pid={}",
                compute_started.elapsed().as_millis(),
                compute_ms,
                count_ms,
                key.0,
                key.1,
                key.2,
                std::process::id()
            );
        }
        *self.cache.borrow_mut() = Some(CachedLayout {
            key,
            snapshot: snapshot.clone(),
        });
        Some(snapshot)
    }

    fn computed_style_for_node(
        &self,
        document: &Document,
        node: NodeId,
    ) -> Option<ComputedStyle> {
        if matches!(document.get(node), Some(NodeData::Document)) {
            return None;
        }

        let layout_key = (
            document.revision(),
            document.style_revision(),
            *self.inputs_revision.borrow(),
        );
        if let Some(cached) = self.cache.borrow().as_ref() {
            if cached.key == layout_key {
                if let Some(style) = cached.snapshot.style(node) {
                    return Some(style.clone());
                }
            }
        }

        let node_key = (
            document.style_revision(),
            *self.inputs_revision.borrow(),
        );
        if let Some(c) = self.full_page_styles.borrow().as_ref() {
            if c.key == node_key {
                if let Some(style) = c.styles.get(&node) {
                    return Some(style.clone());
                }
            }
        }
        let mut cached_guard = self.node_styles.borrow_mut();
        let cache = match cached_guard.as_mut() {
            Some(c) if c.key == node_key => c,
            _ => {
                *cached_guard = Some(CachedNodeStyles {
                    key: node_key,
                    styles: Rc::new(HashMap::new()),
                });
                cached_guard.as_mut().unwrap()
            }
        };

        if let Some(style) = cache.styles.get(&node) {
            return Some(style.clone());
        }

        let mut chain = Vec::new();
        let mut curr = Some(node);
        let mut visited = std::collections::HashSet::new();
        while let Some(n) = curr {
            if !visited.insert(n) || matches!(document.get(n), Some(NodeData::Document)) {
                break;
            }
            chain.push(n);
            curr = composed_parent(document, n);
        }
        chain.reverse();

        let environment = self.style_environment(document);
        let mut current_parent_style: Option<ComputedStyle> = None;
        let styles_map = Rc::make_mut(&mut cache.styles);
        for n in chain {
            if let Some(style) = styles_map.get(&n) {
                current_parent_style = Some(style.clone());
            } else {
                let style = compute_page_style_for_node(
                    document,
                    &environment,
                    n,
                    current_parent_style.as_ref(),
                );
                styles_map.insert(n, style.clone());
                current_parent_style = Some(style);
            }
        }

        current_parent_style
    }

    fn computed_style(
        &self,
        document: &Document,
        snapshot: &LayoutSnapshot,
        node: NodeId,
    ) -> Option<ComputedStyle> {
        if let Some(style) = snapshot.style(node) {
            return Some(style.clone());
        }
        self.computed_style_for_node(document, node)
    }

    fn viewport_size(&self) -> (f32, f32) {
        self.inputs.borrow().viewport
    }

    fn scroll_position(&self) -> (f32, f32) {
        self.inputs.borrow().scroll
    }

    fn request_scroll_position(&self, x: f32, y: f32) {
        self.set_scroll(x, y);
        self.pending_scroll_request.set(Some((x, y)));
    }

    fn rendering_revision(&self) -> (u64, u64) {
        (*self.inputs_revision.borrow(), self.scroll_revision.get())
    }
}
