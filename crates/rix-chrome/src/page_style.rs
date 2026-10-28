
use rix_core::url::RixUrl;
use rix_css::computed::{
    compute_style_full, compute_tree_styles_with_scoped_author, ComputedStyle, LayoutContext,
};
use rix_css::rule_index::RuleIndex;
use rix_dom::node::NodeData;
use rix_dom::tree::Document;
use rix_dom::NodeId;
use rix_layout::taffy_adapter::TaffyLayoutEngine;
use rix_layout::tree::LayoutTree;
use rix_text::font_loader::FontDatabase;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use rayon::prelude::*;

pub fn resolve_external_resource(base_url: &str, reference: &str) -> Option<(String, RixUrl)> {
    let target = RixUrl::parse(base_url).ok()?.join(reference.trim()).ok()?;
    matches!(
        target.scheme(),
        rix_core::url::RixScheme::Http | rix_core::url::RixScheme::Https
    )
    .then(|| (target.to_string(), target))
}

pub fn document_base_url(document: &Document, document_url: &str) -> String {
    for node_id in document.descendants(document.root()) {
        if let Some(NodeData::Element(element)) = document.get(node_id) {
            if &*element.name.local == "base" {
                if let Some(href) = element.get_attribute("href") {
                    if let Some((resolved, _)) = resolve_external_resource(document_url, href) {
                        return resolved;
                    }
                }
                break;
            }
        }
    }
    document_url.to_string()
}

pub fn resolve_page_link(base_url: &str, href: &str) -> Option<String> {
    let target = RixUrl::parse(base_url).ok()?.join(href.trim()).ok()?;
    match target.scheme() {
        rix_core::url::RixScheme::Http
        | rix_core::url::RixScheme::Https
        | rix_core::url::RixScheme::Rix
        | rix_core::url::RixScheme::About => Some(target.to_string()),
        _ => None,
    }
}

#[derive(Clone)]
struct CachedAuthorIndex {
    sheet_hashes: Vec<u64>,
    index: RuleIndex,
}

thread_local! {
    static STYLE_ENV_CACHE: std::cell::RefCell<Option<std::rc::Rc<StyleEnvironment>>> = const { std::cell::RefCell::new(None) };
    static CACHED_AUTHOR_INDEX: std::cell::RefCell<Option<CachedAuthorIndex>> = const { std::cell::RefCell::new(None) };
}

pub fn compute_page_layout(
    document: &Document,
    document_url: &str,
    external_stylesheets: &HashMap<String, String>,
    viewport_width: f32,
    viewport_height: f32,
    fonts: &FontDatabase,
) -> LayoutTree {
    compute_page_styles_and_layout(
        document,
        document_url,
        external_stylesheets,
        viewport_width,
        viewport_height,
        fonts,
    )
    .0
}

pub fn compute_page_styles_and_layout(
    document: &Document,
    document_url: &str,
    external_stylesheets: &HashMap<String, String>,
    viewport_width: f32,
    viewport_height: f32,
    fonts: &FontDatabase,
) -> (LayoutTree, HashMap<NodeId, ComputedStyle>) {
    let sources = collect_style_sources(
        document,
        document_url,
        external_stylesheets,
        viewport_width,
        viewport_height,
    );
    let environment = STYLE_ENV_CACHE.with(|cache| {
        let mut guard = cache.borrow_mut();
        if let Some(existing) = guard.as_ref() {
            if existing.signature == sources.signature {
                return existing.clone();
            }
        }
        let built = std::rc::Rc::new(StyleEnvironment::build_from_sources(
            sources,
            viewport_width,
            viewport_height,
        ));
        *guard = Some(built.clone());
        built
    });
    compute_page_styles_and_layout_with_environment(document, &environment, fonts)
}

pub fn compute_page_styles_and_layout_with_environment(
    document: &Document,
    environment: &StyleEnvironment,
    fonts: &FontDatabase,
) -> (LayoutTree, HashMap<NodeId, ComputedStyle>) {
    let trace = std::env::var_os("RIX_WORKER_TRACE").is_some();
    let phase_started = std::time::Instant::now();
    let mut computed = HashMap::with_capacity(document.node_count().max(1024));
    compute_tree_styles_with_scoped_author(
        document,
        document.root(),
        None,
        &environment.ua_index,
        &environment.author_index,
        &environment.shadow_author_indexes,
        &environment.layout_context,
        None,
        &mut computed,
    );
    let cascade_done = std::time::Instant::now();
    let mut layout = LayoutTree::from_dom(document, document.root(), &computed);
    let mut engine = TaffyLayoutEngine::new();
    if let Err(error) = engine.compute_layout_with_fonts(
        &mut layout,
        environment.layout_context.viewport_width,
        environment.layout_context.viewport_height,
        fonts,
    ) {
        tracing::warn!("Page layout failed: {error}");
    }
    if trace {
        eprintln!(
            "[rix-style] cascade={}ms layout={}ms rules={} nodes={} pid={}",
            cascade_done
                .saturating_duration_since(phase_started)
                .as_millis(),
            cascade_done.elapsed().as_millis(),
            environment.author_index.len(),
            computed.len(),
            std::process::id()
        );
    }
    (layout, computed)
}

pub(crate) fn compute_page_style_for_node(
    document: &Document,
    environment: &StyleEnvironment,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
) -> ComputedStyle {
    let scope_author_index = document
        .shadow_root_containing(node)
        .and_then(|root| environment.shadow_author_indexes.get(&root))
        .unwrap_or(&environment.author_index);
    let host_author_index = match document.get(node) {
        Some(NodeData::Element(element)) => element
            .shadow_root
            .and_then(|root| environment.shadow_author_indexes.get(&root)),
        _ => None,
    };
    let slotted_rules = if let Some(slot) = document.assigned_slot_for(node) {
        if let Some(slot_root) = document.shadow_root_containing(slot) {
            if let Some(shadow_index) = environment.shadow_author_indexes.get(&slot_root) {
                shadow_index.match_slotted_element_with_container(
                    document,
                    node,
                    slot,
                    environment.layout_context.viewport_width,
                    environment.layout_context.viewport_height,
                    environment.layout_context.viewport_width,
                    environment.layout_context.viewport_height,
                    None,
                )
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    compute_style_full(
        document,
        node,
        parent_style,
        &environment.ua_index,
        scope_author_index,
        host_author_index,
        &slotted_rules,
        &environment.layout_context,
        None,
    )
}

pub(crate) struct CollectedStyleSources<'a> {
    document: Vec<&'a str>,
    shadow: Vec<(NodeId, Vec<&'a str>)>,
    signature: u64,
}

fn collect_style_sources<'a>(
    document: &'a Document,
    document_url: &str,
    external_stylesheets: &'a HashMap<String, String>,
    viewport_width: f32,
    viewport_height: f32,
) -> CollectedStyleSources<'a> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut doc_hasher = std::collections::hash_map::DefaultHasher::new();
    viewport_width.to_bits().hash(&mut hasher);
    viewport_height.to_bits().hash(&mut hasher);
    (rix_css::preferred_color_scheme() as u8).hash(&mut hasher);

    let base_url = document_base_url(document, document_url);
    let mut document_rules: Vec<&str> = Vec::new();

    for node_id in document.descendants(document.root()) {
        let Some(NodeData::Element(element)) = document.get(node_id) else {
            continue;
        };
        if &*element.name.local == "style" {
            if in_noscript(document, node_id) {
                continue;
            }
            if let Some(css) = document.dynamic_stylesheet(node_id) {
                node_id.hash(&mut doc_hasher);
                css.hash(&mut doc_hasher);
                document_rules.push(css);
            } else {
                for child_id in document.children(node_id) {
                    if let Some(NodeData::Text(css)) = document.get(child_id) {
                        node_id.hash(&mut doc_hasher);
                        css.hash(&mut doc_hasher);
                        document_rules.push(css);
                    }
                }
            }
        } else if &*element.name.local == "link"
            && element
                .get_attribute("rel")
                .unwrap_or("")
                .split_ascii_whitespace()
                .any(|token| token.eq_ignore_ascii_case("stylesheet"))
        {
            if let Some(href) = element.get_attribute("href") {
                if let Some(stylesheet_url) = resolve_page_link(&base_url, href) {
                    if let Some(css) = external_stylesheets.get(&stylesheet_url) {
                        stylesheet_url.hash(&mut doc_hasher);
                        css.hash(&mut doc_hasher);
                        document_rules.push(css);
                    }
                }
            }
        }
    }

    for css in document.adopted_stylesheets(document.root()) {
        css.hash(&mut doc_hasher);
        document_rules.push(css);
    }

    let document_signature = doc_hasher.finish();
    document_signature.hash(&mut hasher);

    let mut shadow_rules: Vec<(NodeId, Vec<&str>)> = Vec::new();
    for shadow_root in shadow_roots(document) {
        shadow_root.hash(&mut hasher);
        let mut rules: Vec<&str> = Vec::new();
        for css in document.adopted_stylesheets(shadow_root) {
            css.hash(&mut hasher);
            rules.push(css);
        }
        for node_id in document.descendants(shadow_root) {
            let Some(NodeData::Element(element)) = document.get(node_id) else {
                continue;
            };
            if &*element.name.local != "style" || in_noscript(document, node_id) {
                continue;
            }
            if let Some(css) = document.dynamic_stylesheet(node_id) {
                node_id.hash(&mut hasher);
                css.hash(&mut hasher);
                rules.push(css);
            } else {
                for child_id in document.children(node_id) {
                    if let Some(NodeData::Text(css)) = document.get(child_id) {
                        node_id.hash(&mut hasher);
                        css.hash(&mut hasher);
                        rules.push(css);
                    }
                }
            }
        }
        shadow_rules.push((shadow_root, rules));
    }

    CollectedStyleSources {
        document: document_rules,
        shadow: shadow_rules,
        signature: hasher.finish(),
    }
}

fn in_noscript(document: &Document, node: NodeId) -> bool {
    document.ancestors(node).any(|ancestor| {
        matches!(document.get(ancestor), Some(NodeData::Element(parent)) if &*parent.name.local == "noscript")
    })
}

pub struct StyleEnvironment {
    signature: u64,
    pub(crate) ua_index: RuleIndex,
    pub(crate) author_index: RuleIndex,
    pub(crate) shadow_author_indexes: HashMap<NodeId, RuleIndex>,
    pub(crate) layout_context: LayoutContext,
}

impl StyleEnvironment {
    pub fn build(
        document: &Document,
        document_url: &str,
        external_stylesheets: &HashMap<String, String>,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Self {
        let t0 = std::time::Instant::now();
        let sources = collect_style_sources(
            document,
            document_url,
            external_stylesheets,
            viewport_width,
            viewport_height,
        );
        let collect_ms = t0.elapsed().as_millis();
        let t1 = std::time::Instant::now();
        let result = Self::build_from_sources(sources, viewport_width, viewport_height);
        let build_ms = t1.elapsed().as_millis();
        if std::env::var_os("RIX_WORKER_TRACE").is_some() && (collect_ms > 20 || build_ms > 20) {
            eprintln!("[rix-style-env] collect={}ms build={}ms", collect_ms, build_ms);
        }
        result
    }

    pub(crate) fn build_from_sources(
        sources: CollectedStyleSources<'_>,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Self {
        let t_author_start = std::time::Instant::now();
        let mut hit_type = "miss";
        let author_index = CACHED_AUTHOR_INDEX.with(|cache| {
            use std::hash::{Hash, Hasher};
            let mut guard = cache.borrow_mut();
            let current_hashes: Vec<u64> = sources
                .document
                .iter()
                .map(|css| {
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    css.hash(&mut h);
                    h.finish()
                })
                .collect();

            if let Some(existing) = guard.as_mut() {
                if existing.sheet_hashes == current_hashes {
                    hit_type = "exact";
                    return existing.index.clone();
                }
                if current_hashes.len() > existing.sheet_hashes.len()
                    && current_hashes.starts_with(&existing.sheet_hashes)
                {
                    hit_type = "prefix";
                    let mut index = existing.index.clone();
                    for &css in &sources.document[existing.sheet_hashes.len()..] {
                        let sheet = rix_css::StyleSheet::parse(css);
                        index.add_stylesheet(&sheet);
                    }
                    *existing = CachedAuthorIndex {
                        sheet_hashes: current_hashes,
                        index: index.clone(),
                    };
                    return index;
                }
            }

            let parsed_document: Vec<Arc<rix_css::StyleSheet>> =
                sources.document.par_iter().map(|css| rix_css::StyleSheet::parse(css)).collect();
            let mut index = RuleIndex::new();
            for sheet in &parsed_document {
                index.add_stylesheet(sheet);
            }
            *guard = Some(CachedAuthorIndex {
                sheet_hashes: current_hashes,
                index: index.clone(),
            });
            index
        });
        if std::env::var_os("RIX_WORKER_TRACE").is_some() {
            eprintln!("[rix-author-index] elapsed={}ms hit={hit_type} sheets={}", t_author_start.elapsed().as_millis(), sources.document.len());
        }

        let mut shadow_author_indexes = HashMap::with_capacity(sources.shadow.len());
        let mut shadow_rule_index_cache: HashMap<u64, RuleIndex> = HashMap::new();
        for (shadow_root, rules) in &sources.shadow {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for css in rules {
                css.hash(&mut hasher);
            }
            let rules_hash = hasher.finish();
            let index = shadow_rule_index_cache
                .entry(rules_hash)
                .or_insert_with(|| {
                    let mut index = RuleIndex::new();
                    for css in rules {
                        let sheet = rix_css::StyleSheet::parse(css);
                        index.add_stylesheet(&sheet);
                    }
                    index
                })
                .clone();
            shadow_author_indexes.insert(*shadow_root, index);
        }

        let ua_index = RuleIndex::default_ua();
        Self {
            signature: sources.signature,
            ua_index,
            author_index,
            shadow_author_indexes,
            layout_context: LayoutContext {
                root_font_size: 16.0,
                viewport_width: viewport_width.max(1.0),
                viewport_height: viewport_height.max(1.0),
            },
        }
    }

    pub fn matches(
        &self,
        document: &Document,
        document_url: &str,
        external_stylesheets: &HashMap<String, String>,
        viewport_width: f32,
        viewport_height: f32,
    ) -> bool {
        let sources = collect_style_sources(
            document,
            document_url,
            external_stylesheets,
            viewport_width,
            viewport_height,
        );
        self.signature == sources.signature
    }
}

fn shadow_roots(document: &Document) -> Vec<NodeId> {
    let mut roots = Vec::new();
    let mut pending = vec![document.root()];
    let mut visited = HashSet::new();
    while let Some(node) = pending.pop() {
        if !visited.insert(node) {
            continue;
        }
        if let Some(NodeData::Element(element)) = document.get(node) {
            if let Some(root) = element.shadow_root {
                roots.push(root);
                pending.push(root);
            }
        }
        pending.extend(document.children(node));
    }
    roots
}
