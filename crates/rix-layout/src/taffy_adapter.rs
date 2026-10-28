
use crate::box_model::{EdgeSizes, Rect};
use crate::inline::{InlineFlowItem, InlineFormattingContext};
use crate::inline_layout::{
    apply_distribution, compute_distribution, compute_paragraph, content_align_offset,
    supports_context, AtomicSize,
};
use crate::isolated::layout_isolated;
use crate::tree::{AnonymousBoxType, LayoutBoxData, LayoutBoxId, LayoutTree};
use rix_css::computed::{ComputedStyle, Display};
use rix_text::{
    FontDatabase, FontDescriptor, FontId, FontStyle, FontWeight, ParagraphLayout, ParagraphOptions,
    SpaceBreakBehavior, TextAlign, TextSpan, TrailingSpaceBehavior,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use taffy::prelude::*;
use taffy::style::Overflow;
use unicode_segmentation::UnicodeSegmentation;

pub struct TaffyLayoutEngine {
    taffy: TaffyTree<LayoutBoxId>,
    fallback_fonts: Option<FontDatabase>,
}

thread_local! {
    static SHARED_TEXT_CACHE: std::cell::RefCell<TextLayoutCache> = std::cell::RefCell::new(TextLayoutCache::default());
}

const MAX_CACHED_PARAGRAPHS: usize = 4096;
const MAX_PARAGRAPH_CACHE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CACHED_INLINE_PARAGRAPHS: usize = 4096;

struct FontSetIdentity {
    faces: Vec<(Arc<Vec<u8>>, u32)>,
    fallbacks: [Option<FontId>; 4],
}

impl FontSetIdentity {
    fn fallbacks(db: &FontDatabase) -> [Option<FontId>; 4] {
        [
            db.default_sans_serif,
            db.default_serif,
            db.default_monospace,
            db.default_emoji,
        ]
    }

    fn new(db: &FontDatabase) -> Self {
        Self {
            faces: (0..db.len())
                .filter_map(|id| db.get_font(FontId(id)))
                .map(|font| (font.bytes.clone(), font.face_index))
                .collect(),
            fallbacks: Self::fallbacks(db),
        }
    }

    fn matches(&self, db: &FontDatabase) -> bool {
        self.faces.len() == db.len()
            && self.fallbacks == Self::fallbacks(db)
            && self.faces.iter().enumerate().all(|(id, (bytes, index))| {
                db.get_font(FontId(id)).is_some_and(|font| {
                    *index == font.face_index && Arc::ptr_eq(bytes, &font.bytes)
                })
            })
    }
}

#[derive(Default)]
pub(crate) struct TextLayoutCache {
    fonts: Option<FontSetIdentity>,
    entries: HashMap<u64, CachedParagraph>,
    bytes: usize,
    use_counter: u64,
}

struct CachedParagraph {
    text: String,
    family: String,
    white_space: String,
    align: String,
    paragraph: Arc<ParagraphLayout>,
    bytes: usize,
    last_use: u64,
}

impl TextLayoutCache {
    fn prepare_fonts(&mut self, db: &FontDatabase) {
        if self.fonts.as_ref().is_some_and(|fonts| fonts.matches(db)) {
            return;
        }
        self.entries.clear();
        self.bytes = 0;
        self.use_counter = 0;
        self.fonts = Some(FontSetIdentity::new(db));
    }

    pub(crate) fn paragraph(
        &mut self,
        box_data: &LayoutBoxData,
        width: Option<f32>,
        db: &FontDatabase,
        apply_ellipsis: bool,
    ) -> Arc<ParagraphLayout> {
        let AnonymousBoxType::Text(text) = &box_data.box_type else {
            return Arc::new(text_layout_for_box(box_data, width, db, apply_ellipsis));
        };
        let style = &box_data.style;
        let ellipsis = apply_ellipsis
            && matches!(style.white_space.as_str(), "nowrap" | "pre")
            && style.text_overflow == "ellipsis"
            && style.overflow_x != "visible";
        let font_style = match style.font_style.as_str() {
            "italic" => FontStyle::Italic,
            "oblique" => FontStyle::Oblique,
            _ => FontStyle::Normal,
        };
        let font_size_bits = style.font_size.to_bits();
        let line_height_bits = (!style.line_height_normal).then(|| style.line_height.to_bits());
        let color_bits = [style.color.r, style.color.g, style.color.b, style.color.a].map(f32::to_bits);
        let width_bits = width.map(f32::to_bits);

        let hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut hasher);
            style.font_family.hash(&mut hasher);
            style.font_weight.hash(&mut hasher);
            font_style.hash(&mut hasher);
            font_size_bits.hash(&mut hasher);
            line_height_bits.hash(&mut hasher);
            color_bits.hash(&mut hasher);
            style.white_space.hash(&mut hasher);
            style.text_align.hash(&mut hasher);
            width_bits.hash(&mut hasher);
            ellipsis.hash(&mut hasher);
            hasher.finish()
        };

        self.use_counter += 1;
        if let Some(entry) = self.entries.get_mut(&hash) {
            if entry.text == *text
                && entry.family == style.font_family
                && entry.white_space == style.white_space
                && entry.align == style.text_align
            {
                entry.last_use = self.use_counter;
                return entry.paragraph.clone();
            }
        }

        let paragraph = Arc::new(text_layout_for_box(box_data, width, db, ellipsis));
        let bytes = std::mem::size_of::<u64>()
            + text.len()
            + style.font_family.len()
            + style.white_space.len()
            + style.text_align.len()
            + std::mem::size_of::<CachedParagraph>()
            + std::mem::size_of::<ParagraphLayout>()
            + paragraph.lines.capacity() * std::mem::size_of::<rix_text::LayoutLine>()
            + paragraph
                .lines
                .iter()
                .map(|line| {
                    line.glyphs.capacity() * std::mem::size_of::<rix_text::layout::LayoutGlyph>()
                })
                .sum::<usize>();

        if bytes > MAX_PARAGRAPH_CACHE_BYTES {
            return paragraph;
        }

        while self.entries.len() >= MAX_CACHED_PARAGRAPHS
            || self.bytes + bytes > MAX_PARAGRAPH_CACHE_BYTES
        {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_use)
                .map(|(&key, _)| key)
            else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.bytes -= removed.bytes;
            }
        }

        let cached = CachedParagraph {
            text: text.clone(),
            family: style.font_family.clone(),
            white_space: style.white_space.clone(),
            align: style.text_align.clone(),
            paragraph: paragraph.clone(),
            bytes,
            last_use: self.use_counter,
        };
        self.entries.insert(hash, cached);
        self.bytes += bytes;
        paragraph
    }
}

fn map_overflow(value: &str) -> Overflow {
    match value {
        "hidden" => Overflow::Hidden,
        "clip" => Overflow::Clip,
        "auto" | "scroll" => Overflow::Scroll,
        _ => Overflow::Visible,
    }
}

fn text_layout_for_box(
    box_data: &LayoutBoxData,
    max_width: Option<f32>,
    font_db: &FontDatabase,
    apply_ellipsis: bool,
) -> ParagraphLayout {
    let AnonymousBoxType::Text(raw_text) = &box_data.box_type else {
        return ParagraphLayout {
            lines: Vec::new(),
            inline_objects: Vec::new(),
            width: 0.0,
            height: 0.0,
        };
    };
    let style = &box_data.style;
    let descriptor = FontDescriptor::new(&style.font_family)
        .with_weight(FontWeight(style.font_weight))
        .with_style(match style.font_style.as_str() {
            "italic" => FontStyle::Italic,
            "oblique" => FontStyle::Oblique,
            _ => FontStyle::Normal,
        });
    let Some(font_id) = font_db.match_font(&descriptor) else {
        return ParagraphLayout {
            lines: Vec::new(),
            inline_objects: Vec::new(),
            width: 0.0,
            height: 0.0,
        };
    };
    let text = normalize_white_space(raw_text, &style.white_space);
    let color = style.color;
    let mut span = TextSpan::new(text, font_id, style.font_size)
        .with_color([color.r, color.g, color.b, color.a]);
    if !style.line_height_normal {
        span = span.with_line_height(style.line_height);
    }
    let align = match style.text_align.as_str() {
        "center" => TextAlign::Center,
        "right" | "end" => TextAlign::Right,
        "justify" => TextAlign::Justify,
        _ => TextAlign::Left,
    };
    let trailing_space_behavior = match style.white_space.as_str() {
        "pre-wrap" => TrailingSpaceBehavior::Hang,
        "pre" | "break-spaces" => TrailingSpaceBehavior::Preserve,
        _ => TrailingSpaceBehavior::Remove,
    };
    let space_break_behavior = if style.white_space == "break-spaces" {
        SpaceBreakBehavior::AfterEachPreservedSpace
    } else {
        SpaceBreakBehavior::Unicode
    };
    let options = ParagraphOptions {
        wrap: !matches!(style.white_space.as_str(), "nowrap" | "pre"),
        trailing_space_behavior,
        space_break_behavior,
        strut: None,
    };
    let mut paragraph =
        ParagraphLayout::compute_with_options(&[span.clone()], align, max_width, font_db, options);
    if apply_ellipsis
        && !options.wrap
        && style.text_overflow == "ellipsis"
        && style.overflow_x != "visible"
    {
        if let Some(width) = max_width.filter(|width| paragraph.width > *width) {
            let ellipsis_span = TextSpan {
                text: "…".to_string(),
                ..span.clone()
            };
            let ellipsis = ParagraphLayout::compute_with_options(
                &[ellipsis_span],
                TextAlign::Left,
                None,
                font_db,
                options,
            );
            if ellipsis.width > width {
                span.text.clear();
            } else {
                let boundaries: Vec<_> = span
                    .text
                    .grapheme_indices(true)
                    .map(|(byte, _)| byte)
                    .chain(std::iter::once(span.text.len()))
                    .collect();
                let original = span.text.clone();
                let mut low = 0;
                let mut high = boundaries.len();
                while low + 1 < high {
                    let mid = (low + high) / 2;
                    span.text = format!("{}…", &original[..boundaries[mid]]);
                    let candidate = ParagraphLayout::compute_with_options(
                        &[span.clone()],
                        TextAlign::Left,
                        None,
                        font_db,
                        options,
                    );
                    if candidate.width <= width {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                span.text = format!("{}…", &original[..boundaries[low]]);
            }
            paragraph =
                ParagraphLayout::compute_with_options(&[span], align, max_width, font_db, options);
        }
    }
    paragraph
}

fn normalize_white_space(text: &str, white_space: &str) -> String {
    if matches!(white_space, "pre" | "pre-wrap" | "break-spaces") {
        return text.to_string();
    }
    let preserve_breaks = white_space == "pre-line";
    let mut out = String::new();
    let mut pending_space = false;
    for ch in text.chars() {
        if preserve_breaks && ch == '\n' {
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
            pending_space = false;
        } else if matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C') {
            pending_space = !out.is_empty() && !out.ends_with('\n');
        } else {
            if pending_space {
                out.push(' ');
            }
            out.push(ch);
            pending_space = false;
        }
    }
    if out.is_empty() && text.chars().any(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C')) {
        return " ".to_string();
    }
    out
}

pub(crate) fn measure_replaced_element(
    box_data: &LayoutBoxData,
    known: Size<Option<f32>>,
    parent_size: Size<Option<f32>>,
) -> Size<f32> {
    let style = &box_data.style;
    let natural = box_data.intrinsic_size.unwrap_or((0.0, 0.0));
    let ratio = style
        .aspect_ratio
        .or(box_data.intrinsic_ratio)
        .or_else(|| (natural.0 > 0.0 && natural.1 > 0.0).then_some(natural.0 / natural.1));
    let resolve = |length: Option<f32>, percent: Option<f32>, basis: Option<f32>| {
        length.or_else(|| {
            percent
                .zip(basis)
                .map(|(percent, basis)| percent * basis / 100.0)
        })
    };
    let width = known
        .width
        .or_else(|| resolve(style.width, style.width_percent, parent_size.width));
    let height = known
        .height
        .or_else(|| resolve(style.height, style.height_percent, parent_size.height));
    let mut size = match (width, height, ratio) {
        (None, None, Some(ratio)) if box_data.is_svg && box_data.intrinsic_size.is_none() => {
            let max_width = parent_size.width.unwrap_or(300.0).min(300.0).max(0.0);
            let max_height = parent_size.height.unwrap_or(150.0).min(150.0).max(0.0);
            let width = max_width.min(max_height * ratio);
            Size {
                width,
                height: width / ratio,
            }
        }
        (Some(width), Some(height), _) => Size { width, height },
        (Some(width), None, Some(ratio)) => Size {
            width,
            height: width / ratio,
        },
        (None, Some(height), Some(ratio)) => Size {
            width: height * ratio,
            height,
        },
        (width, height, _) => Size {
            width: width.unwrap_or(natural.0),
            height: height.unwrap_or(natural.1),
        },
    };
    let clamp = |value: f32, minimum: Option<f32>, maximum: Option<f32>| {
        value
            .min(maximum.unwrap_or(f32::INFINITY))
            .max(minimum.unwrap_or(0.0))
    };
    size.width = clamp(
        size.width,
        resolve(style.min_width, style.min_width_percent, parent_size.width),
        resolve(style.max_width, style.max_width_percent, parent_size.width),
    );
    if height.is_none() {
        if let Some(ratio) = ratio {
            size.height = size.width / ratio;
        }
    }
    size.height = clamp(
        size.height,
        resolve(
            style.min_height,
            style.min_height_percent,
            parent_size.height,
        ),
        resolve(
            style.max_height,
            style.max_height_percent,
            parent_size.height,
        ),
    );
    if width.is_none() && height.is_some() || width.is_none() && height.is_none() {
        if let Some(ratio) = ratio {
            size.width = clamp(
                size.height * ratio,
                resolve(style.min_width, style.min_width_percent, parent_size.width),
                resolve(style.max_width, style.max_width_percent, parent_size.width),
            );
        }
    }
    size
}

impl Default for TaffyLayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TaffyLayoutEngine {
    pub fn new() -> Self {
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        Self {
            taffy,
            fallback_fonts: None,
        }
    }

    pub fn compute_layout(
        &mut self,
        tree: &mut LayoutTree,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Result<(), String> {
        let fonts = self.fallback_fonts.take().unwrap_or_else(|| {
            let mut fonts = FontDatabase::new();
            fonts.scan_system_fonts();
            fonts
        });
        let result = self.compute_layout_with_fonts(tree, viewport_width, viewport_height, &fonts);
        self.fallback_fonts = Some(fonts);
        result
    }

    pub fn compute_layout_with_fonts(
        &mut self,
        tree: &mut LayoutTree,
        viewport_width: f32,
        viewport_height: f32,
        font_db: &FontDatabase,
    ) -> Result<(), String> {
        let Some(root_id) = tree.root else {
            return Ok(());
        };
        if font_db.is_empty()
            && tree
                .iter()
                .any(|b| matches!(b.box_type, AnonymousBoxType::Text(_)))
        {
            return Err("Text layout requires a loaded font database".to_string());
        }
        self.taffy.clear();
        let mut inline_paragraphs = InlineParagraphCache::default();
        let mut box_to_taffy = HashMap::with_capacity(tree.len());
        let mut fixed_nodes = Vec::new();
        let taffy_html_root = self.build_taffy_tree_recursive(
            tree,
            root_id,
            &mut box_to_taffy,
            false,
            &mut fixed_nodes,
        )?;
        let taffy_root = if fixed_nodes.is_empty() {
            taffy_html_root
        } else {
            let mut viewport_children = Vec::with_capacity(1 + fixed_nodes.len());
            viewport_children.push(taffy_html_root);
            viewport_children.extend(fixed_nodes);
            self.taffy
                .new_with_children(
                    Style {
                        display: taffy::style::Display::Block,
                        size: Size {
                            width: Dimension::length(viewport_width),
                            height: Dimension::length(viewport_height),
                        },
                        ..Default::default()
                    },
                    &viewport_children,
                )
                .map_err(|e| format!("Failed to create Taffy viewport: {e:?}"))?
        };
        SHARED_TEXT_CACHE.with(|cache_cell| {
            let mut text_cache_guard = cache_cell.borrow_mut();
            text_cache_guard.prepare_fonts(font_db);
            let text_cache = &mut *text_cache_guard;
        self.taffy
            .compute_layout_with_measure(
                taffy_root,
                Size {
                    width: AvailableSpace::Definite(viewport_width),
                    height: AvailableSpace::Definite(viewport_height),
                },
                |inputs, _, context, style| {
                    let box_data = context.and_then(|id| tree.get(*id));
                    let mut leaf_style = style.clone();
                    if box_data.is_some_and(|box_data| box_data.is_replaced) {
                        leaf_style.aspect_ratio = None;
                    }
                    let parent_size = inputs.parent_size;
                    let mut output = taffy::compute_leaf_layout(
                        inputs,
                        &leaf_style,
                        |_, _| 0.0,
                        |known, available| {
                            let Some(box_data) = box_data else {
                                return Size::ZERO;
                            };
                            if box_data.is_replaced {
                                return measure_replaced_element(box_data, known, parent_size);
                            }
                            let width = known.width.or(match available.width {
                                AvailableSpace::Definite(w) => Some(w.max(0.0)),
                                AvailableSpace::MinContent => Some(0.0),
                                AvailableSpace::MaxContent => None,
                            });
                            if let Some(context) = tree
                                .inline_context(box_data.id)
                                .filter(|context| supports_context(tree, context))
                            {
                                let containing_height =
                                    definite_content_height(&box_data.style, parent_size.height);
                                let paragraph = inline_paragraphs.get_or_compute(
                                    tree,
                                    context,
                                    width,
                                    containing_height,
                                    text_cache,
                                    font_db,
                                    0,
                                );
                                return paragraph.map_or(Size::ZERO, |paragraph| Size {
                                    width: known.width.unwrap_or(paragraph.width),
                                    height: known.height.unwrap_or(paragraph.height),
                                });
                            }
                            let paragraph = text_cache.paragraph(box_data, width, font_db, false);
                            Size {
                                width: known.width.unwrap_or(paragraph.width),
                                height: known.height.unwrap_or(paragraph.height),
                            }
                        },
                    );
                    if let Some(box_data) = box_data
                        .filter(|box_data| matches!(box_data.box_type, AnonymousBoxType::Text(_)))
                    {
                        let paragraph =
                            text_cache.paragraph(box_data, Some(output.size.width), font_db, false);
                        output.baselines = taffy::tree::Baselines {
                            first: paragraph.lines.first().map(|line| line.baseline),
                            last: paragraph.lines.last().map(|line| line.baseline),
                        };
                    } else if let Some(box_data) = box_data {
                        if let Some(context) = tree
                            .inline_context(box_data.id)
                            .filter(|context| supports_context(tree, context))
                        {
                            let containing_height =
                                definite_content_height(&box_data.style, parent_size.height);
                            let paragraph = inline_paragraphs.get_or_compute(
                                tree,
                                context,
                                Some(output.size.width),
                                containing_height,
                                text_cache,
                                font_db,
                                0,
                            );
                            output.baselines = taffy::tree::Baselines {
                                first: paragraph.as_ref().and_then(|paragraph| {
                                    paragraph.lines.first().map(|line| line.baseline)
                                }),
                                last: paragraph.as_ref().and_then(|paragraph| {
                                    paragraph.lines.last().map(|line| line.baseline)
                                }),
                            };
                        }
                    }
                    output
                },
            )
            .map_err(|e| format!("Taffy layout computation failed: {e:?}"))?;
        self.write_back_coordinates(
            tree,
            root_id,
            0.0,
            0.0,
            viewport_width,
            viewport_width,
            viewport_height,
            &box_to_taffy,
        )?;
        let text_ids: Vec<_> = tree
            .iter()
            .filter(|b| {
                box_to_taffy.contains_key(&b.id) && matches!(b.box_type, AnonymousBoxType::Text(_))
            })
            .map(|b| b.id)
            .collect();
        for id in text_ids {
            let paragraph = {
                let b = tree
                    .get(id)
                    .ok_or_else(|| "Text box disappeared".to_string())?;
                text_cache
                    .paragraph(b, Some(b.dimensions.content.width), font_db, true)
            };
            tree.get_mut(id).unwrap().text_layout = Some(paragraph.as_ref().clone());
        }

        let contexts = tree.inline_contexts().to_vec();
        let atomic_boxes: HashSet<LayoutBoxId> = contexts
            .iter()
            .flat_map(|context| context.items.iter())
            .filter_map(|item| match item {
                InlineFlowItem::Atomic(id) => Some(*id),
                _ => None,
            })
            .collect();
        for context in contexts {
            if !supports_context(tree, &context)
                || has_atomic_ancestor(tree, context.block, &atomic_boxes)
            {
                continue;
            }
            let Some(origin) = tree
                .get(context.block)
                .map(|box_data| box_data.dimensions.content)
            else {
                continue;
            };
            let containing_height = definite_content_height_after_layout(tree, context.block);
            let Some(paragraph) = inline_paragraphs.get_or_compute(
                tree,
                &context,
                Some(origin.width),
                containing_height,
                text_cache,
                font_db,
                0,
            ) else {
                continue;
            };
            let offset = tree.get(context.block).map_or(0.0, |box_data| {
                content_align_offset(&box_data.style, origin.height, paragraph.height)
            });
            let origin = Rect::new(origin.x, origin.y + offset, origin.width, origin.height);
            let distribution = compute_distribution(tree, &context, &paragraph, origin);
            apply_distribution(tree, &distribution);
            for item in &context.items {
                let InlineFlowItem::Atomic(id) = item else {
                    continue;
                };
                let Some(box_data) = tree.get(*id) else {
                    continue;
                };
                if box_data.is_replaced {
                    continue;
                }
                let rect = box_data.dimensions.content;
                if let Some(layout) = layout_isolated(
                    tree,
                    *id,
                    Some(rect.width),
                    containing_height,
                    text_cache,
                    font_db,
                    0,
                ) {
                    crate::isolated::apply_isolated_layout(tree, &layout, rect.x, rect.y);
                }
            }
        }
            Ok(())
        })
    }

    fn build_taffy_tree_recursive(
        &mut self,
        tree: &LayoutTree,
        box_id: LayoutBoxId,
        map: &mut HashMap<LayoutBoxId, NodeId>,
        is_fixed_subtree: bool,
        fixed_nodes: &mut Vec<NodeId>,
    ) -> Result<NodeId, String> {
        let box_data = match tree.get(box_id) {
            Some(b) => b,
            None => return Err(format!("Box ID {:?} not found in tree", box_id)),
        };

        let mut taffy_children = Vec::new();
        let shared_inline_context = tree
            .inline_context(box_id)
            .is_some_and(|context| supports_context(tree, context));
        if !shared_inline_context {
            let mut sorted_children = box_data.children.clone();
            sorted_children.sort_by_key(|&cid| tree.get(cid).map(|b| b.style.order).unwrap_or(0));
            for child_id in sorted_children {
                let child_is_fixed = tree
                    .get(child_id)
                    .is_some_and(|b| b.style.position == "fixed");
                if child_is_fixed && !is_fixed_subtree {
                    let child_node = self.build_taffy_tree_recursive(
                        tree,
                        child_id,
                        map,
                        true,
                        fixed_nodes,
                    )?;
                    fixed_nodes.push(child_node);
                } else {
                    let child_node = self.build_taffy_tree_recursive(
                        tree,
                        child_id,
                        map,
                        is_fixed_subtree,
                        fixed_nodes,
                    )?;
                    taffy_children.push(child_node);
                }
            }
        }

        let mut taffy_style = taffy_style_for_box(&box_data.style, box_data);
        apply_inline_flex_approximation(&mut taffy_style, tree, box_data, shared_inline_context);
        apply_independent_formatting_context(&mut taffy_style, &box_data.style, box_data);

        let node = if matches!(box_data.box_type, AnonymousBoxType::Text(_))
            || box_data.is_replaced
            || shared_inline_context
        {
            self.taffy
                .new_leaf_with_context(taffy_style, box_id)
                .map_err(|e| format!("Failed to create measured text leaf: {e:?}"))?
        } else if taffy_children.is_empty() {
            self.taffy
                .new_leaf(taffy_style)
                .map_err(|e| format!("Failed to create Taffy leaf node: {e:?}"))?
        } else {
            self.taffy
                .new_with_children(taffy_style, &taffy_children)
                .map_err(|e| format!("Failed to create Taffy parent node: {e:?}"))?
        };

        map.insert(box_id, node);
        Ok(node)
    }

    pub(crate) fn convert_style(style: &ComputedStyle) -> Style {
        let display = match style.display {
            Display::Flex | Display::InlineFlex => taffy::style::Display::Flex,
            Display::Grid | Display::InlineGrid => taffy::style::Display::Grid,
            Display::None => taffy::style::Display::None,
            _ => taffy::style::Display::Block,
        };

        let width = match (style.width, style.width_percent) {
            (Some(w), _) => Dimension::length(w),
            (None, Some(pct)) => Dimension::percent(pct / 100.0),
            (None, None) => Dimension::auto(),
        };

        let height = match (style.height, style.height_percent) {
            (Some(h), _) => Dimension::length(h),
            (None, Some(pct)) => Dimension::percent(pct / 100.0),
            (None, None) => Dimension::auto(),
        };

        let min_size = Size {
            width: match (style.min_width, style.min_width_percent) {
                (Some(w), _) => LengthPercentageAuto::length(w),
                (None, Some(pct)) => LengthPercentageAuto::percent(pct / 100.0),
                (None, None) => LengthPercentageAuto::auto(),
            },
            height: match (style.min_height, style.min_height_percent) {
                (Some(h), _) => LengthPercentageAuto::length(h),
                (None, Some(pct)) => LengthPercentageAuto::percent(pct / 100.0),
                (None, None) => LengthPercentageAuto::auto(),
            },
        };

        let max_size = Size {
            width: match (style.max_width, style.max_width_percent) {
                (Some(w), _) => LengthPercentageAuto::length(w),
                (None, Some(pct)) => LengthPercentageAuto::percent(pct / 100.0),
                (None, None) => LengthPercentageAuto::auto(),
            },
            height: match (style.max_height, style.max_height_percent) {
                (Some(h), _) => LengthPercentageAuto::length(h),
                (None, Some(pct)) => LengthPercentageAuto::percent(pct / 100.0),
                (None, None) => LengthPercentageAuto::auto(),
            },
        };

        let inset = taffy::geometry::Rect {
            left: match (style.left, style.left_percent) {
                (Some(value), _) => LengthPercentageAuto::length(value),
                (None, Some(percent)) => LengthPercentageAuto::percent(percent / 100.0),
                (None, None) => LengthPercentageAuto::auto(),
            },
            right: match (style.right, style.right_percent) {
                (Some(value), _) => LengthPercentageAuto::length(value),
                (None, Some(percent)) => LengthPercentageAuto::percent(percent / 100.0),
                (None, None) => LengthPercentageAuto::auto(),
            },
            top: match (style.top, style.top_percent) {
                (Some(value), _) => LengthPercentageAuto::length(value),
                (None, Some(percent)) => LengthPercentageAuto::percent(percent / 100.0),
                (None, None) => LengthPercentageAuto::auto(),
            },
            bottom: match (style.bottom, style.bottom_percent) {
                (Some(value), _) => LengthPercentageAuto::length(value),
                (None, Some(percent)) => LengthPercentageAuto::percent(percent / 100.0),
                (None, None) => LengthPercentageAuto::auto(),
            },
        };

        let padding_side = |percent: Option<f32>, px: f32| -> LengthPercentage {
            match percent {
                Some(percent) => LengthPercentage::percent(percent / 100.0),
                None => LengthPercentage::length(px),
            }
        };
        let padding = taffy::geometry::Rect {
            left: padding_side(style.padding_left_percent, style.padding_left),
            right: padding_side(style.padding_right_percent, style.padding_right),
            top: padding_side(style.padding_top_percent, style.padding_top),
            bottom: padding_side(style.padding_bottom_percent, style.padding_bottom),
        };

        let margin = taffy::geometry::Rect {
            left: if style.margin_left_auto {
                LengthPercentageAuto::auto()
            } else {
                LengthPercentageAuto::length(style.margin_left)
            },
            right: if style.margin_right_auto {
                LengthPercentageAuto::auto()
            } else {
                LengthPercentageAuto::length(style.margin_right)
            },
            top: if style.margin_top_auto {
                LengthPercentageAuto::auto()
            } else {
                LengthPercentageAuto::length(style.margin_top)
            },
            bottom: if style.margin_bottom_auto {
                LengthPercentageAuto::auto()
            } else {
                LengthPercentageAuto::length(style.margin_bottom)
            },
        };

        let flex_direction = match style.flex_direction.as_str() {
            "column" => taffy::style::FlexDirection::Column,
            "column-reverse" => taffy::style::FlexDirection::ColumnReverse,
            "row-reverse" => taffy::style::FlexDirection::RowReverse,
            _ => taffy::style::FlexDirection::Row,
        };

        let align_items = match style.align_items.as_deref() {
            Some("center") => Some(AlignItems::CENTER),
            Some("flex-start") | Some("start") => Some(AlignItems::FLEX_START),
            Some("flex-end") | Some("end") => Some(AlignItems::FLEX_END),
            Some("stretch") => Some(AlignItems::STRETCH),
            Some("baseline") => Some(AlignItems::BASELINE),
            _ => None,
        };

        let justify_content = match style.justify_content.as_deref() {
            Some("center") => Some(AlignContent::CENTER),
            Some("flex-start") | Some("start") => Some(AlignContent::FLEX_START),
            Some("flex-end") | Some("end") => Some(AlignContent::FLEX_END),
            Some("space-between") => Some(AlignContent::SPACE_BETWEEN),
            Some("space-around") => Some(AlignContent::SPACE_AROUND),
            Some("space-evenly") => Some(AlignContent::SPACE_EVENLY),
            _ => None,
        };

        let align_self = match style.align_self.as_deref() {
            Some("center") => Some(AlignItems::CENTER),
            Some("flex-start") | Some("start") => Some(AlignItems::FLEX_START),
            Some("flex-end") | Some("end") => Some(AlignItems::FLEX_END),
            Some("stretch") => Some(AlignItems::STRETCH),
            Some("baseline") => Some(AlignItems::BASELINE),
            _ => None,
        };

        let justify_items = match style.justify_items.as_deref() {
            Some("center") => Some(AlignItems::CENTER),
            Some("flex-start") | Some("start") => Some(AlignItems::FLEX_START),
            Some("flex-end") | Some("end") => Some(AlignItems::FLEX_END),
            Some("stretch") => Some(AlignItems::STRETCH),
            Some("baseline") => Some(AlignItems::BASELINE),
            _ => None,
        };

        let justify_self = match style.justify_self.as_deref() {
            Some("center") => Some(AlignItems::CENTER),
            Some("flex-start") | Some("start") => Some(AlignItems::FLEX_START),
            Some("flex-end") | Some("end") => Some(AlignItems::FLEX_END),
            Some("stretch") => Some(AlignItems::STRETCH),
            Some("baseline") => Some(AlignItems::BASELINE),
            _ => None,
        };

        let grid_row = match style.grid_row_start {
            Some(r) => line(r),
            None => Line::default(),
        };

        let grid_column = match style.grid_column_start {
            Some(c) => line(c),
            None => Line::default(),
        };

        let gap = Size {
            width: LengthPercentage::length(style.column_gap.unwrap_or(style.gap)),
            height: LengthPercentage::length(style.row_gap.unwrap_or(style.gap)),
        };

        let position = match style.position.as_str() {
            "absolute" | "fixed" => taffy::style::Position::Absolute,
            _ => taffy::style::Position::Relative,
        };

        let box_sizing = match style.box_sizing.as_str() {
            "border-box" => taffy::style::BoxSizing::BorderBox,
            _ => taffy::style::BoxSizing::ContentBox,
        };

        let flex_wrap = match style.flex_wrap.as_str() {
            "wrap" => taffy::style::FlexWrap::Wrap,
            "wrap-reverse" => taffy::style::FlexWrap::WrapReverse,
            _ => taffy::style::FlexWrap::NoWrap,
        };

        Style {
            display,
            position,
            box_sizing,
            flex_direction,
            flex_wrap,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            flex_basis: match (style.flex_basis, style.flex_basis_percent) {
                (Some(basis), _) => Dimension::length(basis),
                (None, Some(percent)) => Dimension::percent(percent / 100.0),
                (None, None) => Dimension::auto(),
            },
            align_items,
            justify_content,
            align_self,
            justify_items,
            justify_self,
            grid_row,
            grid_column,
            grid_template_columns: convert_grid_template(&style.grid_template_columns),
            grid_template_rows: convert_grid_template(&style.grid_template_rows),
            grid_template_column_names: style
                .grid_template_columns
                .line_names
                .iter()
                .map(|names| names.iter().map(|name| name.clone().into()).collect())
                .collect(),
            grid_template_row_names: style
                .grid_template_rows
                .line_names
                .iter()
                .map(|names| names.iter().map(|name| name.clone().into()).collect())
                .collect(),
            gap,
            size: Size { width, height },
            aspect_ratio: style.aspect_ratio,
            min_size,
            max_size,
            inset,
            padding,
            margin,
            border: taffy::geometry::Rect {
                top: LengthPercentage::length(style.border_top_width),
                right: LengthPercentage::length(style.border_right_width),
                bottom: LengthPercentage::length(style.border_bottom_width),
                left: LengthPercentage::length(style.border_left_width),
            },
            overflow: taffy::geometry::Point {
                x: map_overflow(&style.overflow_x),
                y: map_overflow(&style.overflow_y),
            },
            ..Default::default()
        }
    }

    fn write_back_coordinates(
        &self,
        tree: &mut LayoutTree,
        box_id: LayoutBoxId,
        parent_x: f32,
        parent_y: f32,
        containing_width: f32,
        viewport_width: f32,
        viewport_height: f32,
        map: &HashMap<LayoutBoxId, NodeId>,
    ) -> Result<(), String> {
        let taffy_node = *map
            .get(&box_id)
            .ok_or_else(|| format!("Mapping missing for box {:?}", box_id))?;

        let layout = self
            .taffy
            .layout(taffy_node)
            .map_err(|e| format!("Failed to read Taffy layout: {e:?}"))?;

        let width = layout.size.width;
        let height = layout.size.height;
        let mut abs_x = parent_x + layout.location.x;
        let mut abs_y = parent_y + layout.location.y;

        if let Some(box_data) = tree.get(box_id) {
            if box_data.style.position == "fixed" {
                if (box_data.style.left == Some(0.0) && box_data.style.right == Some(0.0)
                    || box_data.style.margin_left_auto && box_data.style.margin_right_auto)
                    && width > 0.0
                    && width < viewport_width
                {
                    abs_x = (viewport_width - width) / 2.0;
                }
                if (box_data.style.top == Some(0.0) && box_data.style.bottom == Some(0.0)
                    || box_data.style.margin_top_auto && box_data.style.margin_bottom_auto)
                    && height > 0.0
                    && height < viewport_height
                {
                    abs_y = (viewport_height - height) / 2.0;
                }
            }
        }

        let (children, content_w) = {
            let box_mut = tree
                .get_mut(box_id)
                .ok_or_else(|| format!("Box ID {:?} not found", box_id))?;

            let padding_side = |percent: Option<f32>, px: f32| match percent {
                Some(percent) => percent / 100.0 * containing_width,
                None => px,
            };
            box_mut.dimensions.padding.top =
                padding_side(box_mut.style.padding_top_percent, box_mut.style.padding_top);
            box_mut.dimensions.padding.right =
                padding_side(box_mut.style.padding_right_percent, box_mut.style.padding_right);
            box_mut.dimensions.padding.bottom = padding_side(
                box_mut.style.padding_bottom_percent,
                box_mut.style.padding_bottom,
            );
            box_mut.dimensions.padding.left =
                padding_side(box_mut.style.padding_left_percent, box_mut.style.padding_left);

            let p_left = box_mut.dimensions.padding.left;
            let p_top = box_mut.dimensions.padding.top;
            let p_right = box_mut.dimensions.padding.right;
            let p_bottom = box_mut.dimensions.padding.bottom;

            let b_left = box_mut.dimensions.border.left;
            let b_top = box_mut.dimensions.border.top;
            let b_right = box_mut.dimensions.border.right;
            let b_bottom = box_mut.dimensions.border.bottom;

            let content_x = abs_x + b_left + p_left;
            let content_y = abs_y + b_top + p_top;
            let content_w = (width - b_left - b_right - p_left - p_right).max(0.0);
            let content_h = (height - b_top - b_bottom - p_top - p_bottom).max(0.0);

            box_mut.dimensions.content = Rect::new(content_x, content_y, content_w, content_h);
            box_mut.dimensions.margin.top = layout.margin.top;
            box_mut.dimensions.margin.right = layout.margin.right;
            box_mut.dimensions.margin.bottom = layout.margin.bottom;
            box_mut.dimensions.margin.left = layout.margin.left;
            (box_mut.children.clone(), content_w)
        };

        for child_id in children {
            if !map.contains_key(&child_id) {
                continue;
            }
            let child_is_fixed = tree
                .get(child_id)
                .is_some_and(|child| child.style.position == "fixed");
            let (origin_x, origin_y, child_containing_width) = if child_is_fixed {
                (0.0, 0.0, viewport_width)
            } else {
                (abs_x, abs_y, content_w)
            };
            self.write_back_coordinates(
                tree,
                child_id,
                origin_x,
                origin_y,
                child_containing_width,
                viewport_width,
                viewport_height,
                map,
            )?;
        }

        Ok(())
    }
}

pub(crate) fn taffy_style_for_box(style: &ComputedStyle, box_data: &LayoutBoxData) -> Style {
    let mut taffy_style = TaffyLayoutEngine::convert_style(style);
    taffy_style.item_is_replaced = box_data.is_replaced;
    if box_data.is_replaced && taffy_style.aspect_ratio.is_none() {
        taffy_style.aspect_ratio = box_data
            .intrinsic_ratio
            .or_else(|| box_data.intrinsic_size.map(|(width, height)| width / height));
    }
    taffy_style
}

pub(crate) fn apply_independent_formatting_context(
    taffy_style: &mut Style,
    style: &ComputedStyle,
    box_data: &LayoutBoxData,
) {
    if taffy_style.display != taffy::style::Display::Block || style.align_content != "normal" {
        return;
    }
    let establishes_context = style.display == Display::InlineBlock
        || matches!(box_data.box_type, AnonymousBoxType::AnonymousBlock);
    if establishes_context {
        taffy_style.align_content = Some(AlignContent::START);
    }
}

pub(crate) fn apply_inline_flex_approximation(
    taffy_style: &mut Style,
    tree: &LayoutTree,
    box_data: &LayoutBoxData,
    shared_inline_context: bool,
) {
    if shared_inline_context
        || box_data.style.display != Display::Block
        || box_data.children.is_empty()
        || box_data.children.len() <= 1
    {
        return;
    }
    let all_inline = box_data.children.iter().all(|&cid| {
        tree.get(cid).is_some_and(|child| {
            child.style.display.is_inline_level()
                || child.style.display == Display::None
                || matches!(child.box_type, AnonymousBoxType::Text(_))
        })
    });
    if !all_inline {
        return;
    }
    taffy_style.display = taffy::style::Display::Flex;
    taffy_style.flex_direction = taffy::style::FlexDirection::Row;
    taffy_style.flex_wrap = taffy::style::FlexWrap::Wrap;
    if box_data.style.text_align == "center" {
        taffy_style.justify_content = Some(taffy::style::AlignContent::CENTER);
    } else if box_data.style.text_align == "right" {
        taffy_style.justify_content = Some(taffy::style::AlignContent::FLEX_END);
    } else if taffy_style.justify_content.is_none() {
        taffy_style.justify_content = Some(taffy::style::AlignContent::FLEX_START);
    }
    taffy_style.align_items = Some(taffy::style::AlignItems::BASELINE);
}

pub(crate) fn compute_atomic_sizes(
    tree: &LayoutTree,
    context: &InlineFormattingContext,
    width: Option<f32>,
    containing_height: Option<f32>,
    text_cache: &mut TextLayoutCache,
    font_db: &FontDatabase,
    depth: u32,
) -> Option<HashMap<LayoutBoxId, AtomicSize>> {
    let parent_size = Size {
        width,
        height: containing_height,
    };
    let mut atomic_sizes = HashMap::new();
    for item in &context.items {
        let InlineFlowItem::Atomic(id) = item else {
            continue;
        };
        let box_data = tree.get(*id)?;
        let (size, baseline) = if box_data.is_replaced {
            (
                measure_replaced_element(
                    box_data,
                    Size {
                        width: None,
                        height: None,
                    },
                    parent_size,
                ),
                None,
            )
        } else {
            let isolated = layout_isolated(tree, *id, width, containing_height, text_cache, font_db, depth + 1);
            let size = isolated.as_ref().map(|layout| layout.size).unwrap_or(Size {
                width: 0.0,
                height: 0.0,
            });
            let baseline = isolated.and_then(|layout| layout.baseline);
            (size, baseline)
        };
        atomic_sizes.insert(
            *id,
            AtomicSize {
                border_width: size.width,
                border_height: size.height,
                margin: EdgeSizes::new(
                    box_data.style.margin_top,
                    box_data.style.margin_right,
                    box_data.style.margin_bottom,
                    box_data.style.margin_left,
                ),
                baseline,
            },
        );
    }
    Some(atomic_sizes)
}

#[derive(Default)]
pub(crate) struct InlineParagraphCache {
    entries: HashMap<(LayoutBoxId, Option<u32>, Option<u32>), (Arc<ParagraphLayout>, usize)>,
    bytes: usize,
}

impl InlineParagraphCache {
    pub(crate) fn get_or_compute(
        &mut self,
        tree: &LayoutTree,
        context: &InlineFormattingContext,
        width: Option<f32>,
        containing_height: Option<f32>,
        text_cache: &mut TextLayoutCache,
        font_db: &FontDatabase,
        depth: u32,
    ) -> Option<Arc<ParagraphLayout>> {
        let key = (
            context.block,
            width.map(f32::to_bits),
            containing_height.map(f32::to_bits),
        );
        if let Some((paragraph, _)) = self.entries.get(&key) {
            return Some(paragraph.clone());
        }
        let block = tree.get(context.block)?;
        let atomic_sizes = compute_atomic_sizes(
            tree,
            context,
            width,
            containing_height,
            text_cache,
            font_db,
            depth,
        )?;
        let atomic_alignments: HashMap<LayoutBoxId, String> = context
            .items
            .iter()
            .filter_map(|item| match item {
                InlineFlowItem::Atomic(id) => {
                    tree.get(*id).map(|box_data| (*id, box_data.style.vertical_align.clone()))
                }
                _ => None,
            })
            .collect();
        let paragraph = Arc::new(compute_paragraph(
            context,
            &block.style,
            width,
            font_db,
            &atomic_sizes,
            &atomic_alignments,
        )?);
        let bytes = std::mem::size_of::<ParagraphLayout>()
            .saturating_add(
                paragraph
                    .lines
                    .capacity()
                    .saturating_mul(std::mem::size_of::<rix_text::LayoutLine>()),
            )
            .saturating_add(
                paragraph
                    .lines
                    .iter()
                    .map(|line| {
                        line.glyphs
                            .capacity()
                            .saturating_mul(std::mem::size_of::<rix_text::layout::LayoutGlyph>())
                    })
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                paragraph
                    .inline_objects
                    .capacity()
                    .saturating_mul(std::mem::size_of::<rix_text::PositionedInlineObject>()),
            );
        if bytes <= MAX_PARAGRAPH_CACHE_BYTES
            && self.entries.len() < MAX_CACHED_INLINE_PARAGRAPHS
            && self.bytes.saturating_add(bytes) <= MAX_PARAGRAPH_CACHE_BYTES
        {
            self.bytes += bytes;
            self.entries.insert(key, (paragraph.clone(), bytes));
        }
        Some(paragraph)
    }
}

pub(crate) fn definite_content_height(style: &ComputedStyle, parent_height: Option<f32>) -> Option<f32> {
    let resolve = |length: Option<f32>, percent: Option<f32>| {
        length.or_else(|| {
            percent
                .zip(parent_height)
                .map(|(percent, basis)| basis * percent / 100.0)
        })
    };
    let height = resolve(style.height, style.height_percent)?;
    if !height.is_finite() {
        return None;
    }

    let border_padding = style.padding_top
        + style.padding_bottom
        + style.border_top_width
        + style.border_bottom_width;
    let to_content_height = |value: f32| {
        if style.box_sizing == "border-box" {
            (value - border_padding).max(0.0)
        } else {
            value.max(0.0)
        }
    };
    let mut content_height = to_content_height(height);
    if let Some(minimum) = resolve(style.min_height, style.min_height_percent) {
        content_height = content_height.max(to_content_height(minimum));
    }
    if let Some(maximum) = resolve(style.max_height, style.max_height_percent) {
        let maximum = to_content_height(maximum);
        if maximum >= content_height {
            content_height = content_height.min(maximum);
        }
    }
    Some(content_height)
}

fn definite_content_height_after_layout(tree: &LayoutTree, block_id: LayoutBoxId) -> Option<f32> {
    let block = tree.get(block_id)?;
    let parent_height = block
        .parent
        .and_then(|parent_id| definite_content_height_after_layout(tree, parent_id));
    definite_content_height(&block.style, parent_height)?;
    Some(block.dimensions.content.height)
}

fn has_atomic_ancestor(
    tree: &LayoutTree,
    box_id: LayoutBoxId,
    atomic_boxes: &HashSet<LayoutBoxId>,
) -> bool {
    let mut current = tree.get(box_id).and_then(|box_data| box_data.parent);
    while let Some(id) = current {
        if atomic_boxes.contains(&id) {
            return true;
        }
        current = tree.get(id).and_then(|box_data| box_data.parent);
    }
    false
}

fn convert_grid_track(track: rix_css::grid::GridTrack) -> taffy::style::TrackSizingFunction {
    use rix_css::grid::{GridBreadth as B, GridTrack};
    use taffy::style::{
        MaxTrackSizingFunction as Max, MinTrackSizingFunction as Min, TrackSizingFunction,
    };
    let minimum = |value| match value {
        B::Length(value) => Min::length(value),
        B::Percent(value) => Min::percent(value),
        B::MinContent => Min::min_content(),
        B::MaxContent => Min::max_content(),
        B::Auto | B::Flex(_) => Min::auto(),
    };
    let maximum = |value| match value {
        B::Length(value) => Max::length(value),
        B::Percent(value) => Max::percent(value),
        B::Flex(value) => Max::fr(value),
        B::MinContent => Max::min_content(),
        B::MaxContent => Max::max_content(),
        B::Auto => Max::auto(),
    };
    match track {
        GridTrack::Breadth(value) => TrackSizingFunction {
            min: minimum(value),
            max: maximum(value),
        },
        GridTrack::MinMax(min, max) => TrackSizingFunction {
            min: minimum(min),
            max: maximum(max),
        },
        GridTrack::FitContent(value) => TrackSizingFunction {
            min: Min::auto(),
            max: match value {
                B::Length(value) => Max::fit_content_px(value),
                B::Percent(value) => Max::fit_content_percent(value),
                _ => Max::auto(),
            },
        },
    }
}

fn convert_grid_template(
    template: &rix_css::grid::GridTemplate,
) -> Vec<taffy::style::GridTemplateComponent<String>> {
    use rix_css::grid::{GridComponent, GridRepeat};
    use taffy::style::{GridTemplateComponent, GridTemplateRepetition, RepetitionCount};
    template
        .components
        .iter()
        .map(|component| match component {
            GridComponent::Single(track) => {
                GridTemplateComponent::Single(convert_grid_track(*track))
            }
            GridComponent::Repeat {
                count,
                tracks,
                line_names,
            } => GridTemplateComponent::Repeat(GridTemplateRepetition {
                count: match count {
                    GridRepeat::Count(count) => RepetitionCount::Count(*count),
                    GridRepeat::AutoFill => RepetitionCount::AutoFill,
                    GridRepeat::AutoFit => RepetitionCount::AutoFit,
                },
                tracks: tracks.iter().copied().map(convert_grid_track).collect(),
                line_names: line_names.clone(),
            }),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BoxDimensions;
    use rix_css::ComputedStyle;
    use rix_text::FontDatabase;

    #[test]
    fn test_text_layout_cache_lookup_and_hit() {
        let mut cache = TextLayoutCache::default();
        let db = FontDatabase::new();
        cache.prepare_fonts(&db);

        let box_data = LayoutBoxData {
            id: LayoutBoxId(1),
            dom_node: None,
            style: ComputedStyle {
                font_family: "sans-serif".to_string(),
                font_size: 16.0,
                ..ComputedStyle::default()
            },
            box_type: AnonymousBoxType::Text("Hello world".to_string()),
            dimensions: BoxDimensions::default(),
            text_layout: None,
            inline_fragments: Vec::new(),
            is_replaced: false,
            is_svg: false,
            intrinsic_size: None,
            intrinsic_ratio: None,
            parent: None,
            children: Vec::new(),
        };

        let p1 = cache.paragraph(&box_data, Some(200.0), &db, false);
        assert_eq!(cache.entries.len(), 1);

        let p2 = cache.paragraph(&box_data, Some(200.0), &db, false);
        assert_eq!(cache.entries.len(), 1);
        assert!(Arc::ptr_eq(&p1, &p2));
    }

    #[test]
    fn test_fixed_element_viewport_centering() {
        let mut engine = TaffyLayoutEngine::new();
        let mut tree = LayoutTree::new();

        let root_id = tree.push(LayoutBoxData {
            id: LayoutBoxId(0),
            dom_node: None,
            style: ComputedStyle {
                display: Display::Block,
                ..ComputedStyle::default()
            },
            box_type: AnonymousBoxType::AnonymousBlock,
            dimensions: BoxDimensions::default(),
            text_layout: None,
            inline_fragments: Vec::new(),
            is_replaced: false,
            is_svg: false,
            intrinsic_size: None,
            intrinsic_ratio: None,
            parent: None,
            children: vec![LayoutBoxId(1)],
        });

        let dialog_id = tree.push(LayoutBoxData {
            id: LayoutBoxId(1),
            dom_node: None,
            style: ComputedStyle {
                display: Display::Block,
                position: "fixed".to_string(),
                width: Some(400.0),
                height: Some(200.0),
                top: Some(0.0),
                bottom: Some(0.0),
                left: Some(0.0),
                right: Some(0.0),
                margin_top_auto: true,
                margin_bottom_auto: true,
                margin_left_auto: true,
                margin_right_auto: true,
                ..ComputedStyle::default()
            },
            box_type: AnonymousBoxType::AnonymousBlock,
            dimensions: BoxDimensions::default(),
            text_layout: None,
            inline_fragments: Vec::new(),
            is_replaced: false,
            is_svg: false,
            intrinsic_size: None,
            intrinsic_ratio: None,
            parent: Some(root_id),
            children: Vec::new(),
        });
        tree.root = Some(root_id);

        engine.compute_layout(&mut tree, 1000.0, 600.0).unwrap();

        let dialog = tree.get(dialog_id).unwrap();
        assert_eq!(dialog.dimensions.content.x, 300.0);
        assert_eq!(dialog.dimensions.content.y, 200.0);
        assert_eq!(dialog.dimensions.content.width, 400.0);
        assert_eq!(dialog.dimensions.content.height, 200.0);
    }

    #[test]
    fn test_dialog_flex_overflow_scroll_height() {
        let mut engine = TaffyLayoutEngine::new();
        let mut tree = LayoutTree::new();

        let root_id = tree.push(LayoutBoxData {
            id: LayoutBoxId(0),
            dom_node: None,
            style: ComputedStyle {
                display: Display::Block,
                ..ComputedStyle::default()
            },
            box_type: AnonymousBoxType::AnonymousBlock,
            dimensions: BoxDimensions::default(),
            text_layout: None,
            inline_fragments: Vec::new(),
            is_replaced: false,
            is_svg: false,
            intrinsic_size: None,
            intrinsic_ratio: None,
            parent: None,
            children: vec![LayoutBoxId(1)],
        });

        let dialog_id = tree.push(LayoutBoxData {
            id: LayoutBoxId(1),
            dom_node: None,
            style: ComputedStyle {
                display: Display::Flex,
                flex_direction: "column".to_string(),
                position: "fixed".to_string(),
                width: Some(700.0),
                height: None,
                top: Some(0.0),
                bottom: Some(0.0),
                margin_top: 24.0,
                margin_bottom: 24.0,
                margin_left: 40.0,
                margin_right: 40.0,
                ..ComputedStyle::default()
            },
            box_type: AnonymousBoxType::AnonymousBlock,
            dimensions: BoxDimensions::default(),
            text_layout: None,
            inline_fragments: Vec::new(),
            is_replaced: false,
            is_svg: false,
            intrinsic_size: None,
            intrinsic_ratio: None,
            parent: Some(root_id),
            children: vec![LayoutBoxId(2), LayoutBoxId(3)],
        });

        let topbar_id = tree.push(LayoutBoxData {
            id: LayoutBoxId(0),
            dom_node: None,
            style: ComputedStyle {
                display: Display::Flex,
                height: Some(45.0),
                ..ComputedStyle::default()
            },
            box_type: AnonymousBoxType::AnonymousBlock,
            dimensions: BoxDimensions::default(),
            text_layout: None,
            inline_fragments: Vec::new(),
            is_replaced: false,
            is_svg: false,
            intrinsic_size: None,
            intrinsic_ratio: None,
            parent: Some(dialog_id),
            children: Vec::new(),
        });

        let body_id = tree.push(LayoutBoxData {
            id: LayoutBoxId(0),
            dom_node: None,
            style: ComputedStyle {
                display: Display::Block,
                height: Some(600.0),
                ..ComputedStyle::default()
            },
            box_type: AnonymousBoxType::AnonymousBlock,
            dimensions: BoxDimensions::default(),
            text_layout: None,
            inline_fragments: Vec::new(),
            is_replaced: false,
            is_svg: false,
            intrinsic_size: None,
            intrinsic_ratio: None,
            parent: None,
            children: Vec::new(),
        });

        let content_id = tree.push(LayoutBoxData {
            id: LayoutBoxId(0),
            dom_node: None,
            style: ComputedStyle {
                display: Display::Block,
                overflow_y: "auto".to_string(),
                height: None,
                flex_shrink: 1.0,
                ..ComputedStyle::default()
            },
            box_type: AnonymousBoxType::AnonymousBlock,
            dimensions: BoxDimensions::default(),
            text_layout: None,
            inline_fragments: Vec::new(),
            is_replaced: false,
            is_svg: false,
            intrinsic_size: None,
            intrinsic_ratio: None,
            parent: Some(dialog_id),
            children: vec![body_id],
        });

        tree.get_mut(body_id).unwrap().parent = Some(content_id);
        tree.get_mut(dialog_id).unwrap().children = vec![topbar_id, content_id];

        tree.root = Some(root_id);

        engine.compute_layout(&mut tree, 1024.0, 385.0).unwrap();

        let dialog = tree.get(dialog_id).unwrap();
        let content = tree.get(content_id).unwrap();
        assert_eq!(dialog.dimensions.content.width, 700.0);
        assert_eq!(dialog.dimensions.content.height, 337.0);
        assert_eq!(content.dimensions.content.width, 700.0);
        assert!(content.dimensions.content.height > 300.0);
    }
}
