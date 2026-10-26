
use crate::box_model::{EdgeSizes, Rect};
use crate::inline::InlineFlowItem;
use crate::inline_layout::{
    compute_distribution, content_align_offset, supports_context, ParagraphDistribution,
};
use crate::tree::{AnonymousBoxType, LayoutBoxId, LayoutTree};
use rix_text::{FontDatabase, ParagraphLayout};
use std::collections::HashMap;
use taffy::prelude::*;
use taffy::TaffyTree;

use crate::taffy_adapter::{
    apply_independent_formatting_context, apply_inline_flex_approximation, definite_content_height,
    measure_replaced_element, taffy_style_for_box, InlineParagraphCache, TextLayoutCache,
};

const MAX_ATOMIC_DEPTH: u32 = 12;

#[derive(Clone)]
struct IsolatedBox {
    content: Rect,
    margin: EdgeSizes,
    text_layout: Option<ParagraphLayout>,
    inline_fragments: Vec<Rect>,
}

impl Default for IsolatedBox {
    fn default() -> Self {
        Self {
            content: Rect::default(),
            margin: EdgeSizes::ZERO,
            text_layout: None,
            inline_fragments: Vec::new(),
        }
    }
}

pub(crate) struct IsolatedLayout {
    pub(crate) size: Size<f32>,
    boxes: Vec<(LayoutBoxId, IsolatedBox)>,
}

struct IsolatedBuilder<'a> {
    tree: &'a LayoutTree,
    nodes: Vec<(LayoutBoxId, NodeId)>,
    paragraph_blocks: Vec<LayoutBoxId>,
}

impl<'a> IsolatedBuilder<'a> {
    fn build(&mut self, taffy: &mut TaffyTree<LayoutBoxId>, box_id: LayoutBoxId) -> Option<NodeId> {
        let tree = self.tree;
        let box_data = tree.get(box_id)?;

        let shared_inline_context = tree
            .inline_context(box_id)
            .is_some_and(|context| supports_context(tree, context));
        if shared_inline_context {
            self.paragraph_blocks.push(box_id);
        }

        let mut taffy_children = Vec::new();
        if !shared_inline_context {
            let mut sorted_children = box_data.children.clone();
            sorted_children.sort_by_key(|&cid| tree.get(cid).map(|b| b.style.order).unwrap_or(0));
            for child_id in sorted_children {
                taffy_children.push(self.build(taffy, child_id)?);
            }
        }

        let mut taffy_style = taffy_style_for_box(&box_data.style, box_data);
        apply_inline_flex_approximation(&mut taffy_style, tree, box_data, shared_inline_context);
        apply_independent_formatting_context(&mut taffy_style, &box_data.style, box_data);

        let node = if matches!(box_data.box_type, AnonymousBoxType::Text(_))
            || box_data.is_replaced
            || shared_inline_context
        {
            taffy.new_leaf_with_context(taffy_style, box_id).ok()?
        } else if taffy_children.is_empty() {
            taffy.new_leaf(taffy_style).ok()?
        } else {
            taffy.new_with_children(taffy_style, &taffy_children).ok()?
        };
        self.nodes.push((box_id, node));
        Some(node)
    }
}

pub(crate) fn layout_isolated(
    tree: &LayoutTree,
    root: LayoutBoxId,
    available_width: Option<f32>,
    containing_height: Option<f32>,
    text_cache: &mut TextLayoutCache,
    font_db: &FontDatabase,
    depth: u32,
) -> Option<IsolatedLayout> {
    if depth > MAX_ATOMIC_DEPTH {
        return None;
    }
    let root_style = &tree.get(root)?.style;
    let explicit_width = root_style.width.is_some() || root_style.width_percent.is_some();

    let mut taffy: TaffyTree<LayoutBoxId> = TaffyTree::new();
    taffy.disable_rounding();
    let mut builder = IsolatedBuilder {
        tree,
        nodes: Vec::new(),
        paragraph_blocks: Vec::new(),
    };
    let root_node = builder.build(&mut taffy, root)?;

    let mut paragraphs = InlineParagraphCache::default();
    let height_space = containing_height
        .map(AvailableSpace::Definite)
        .unwrap_or(AvailableSpace::MaxContent);

    let mut compute = |taffy: &mut TaffyTree<LayoutBoxId>,
                       width_space: AvailableSpace,
                       paragraphs: &mut InlineParagraphCache|
     -> Option<()> {
        taffy
            .compute_layout_with_measure(
                root_node,
                Size {
                    width: width_space,
                    height: height_space,
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
                                let paragraph = paragraphs.get_or_compute(
                                    tree,
                                    context,
                                    width,
                                    containing_height,
                                    text_cache,
                                    font_db,
                                    depth,
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
                            let paragraph = paragraphs.get_or_compute(
                                tree,
                                context,
                                Some(output.size.width),
                                containing_height,
                                text_cache,
                                font_db,
                                depth,
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
            .ok()?;
        Some(())
    };

    let width_space = if explicit_width {
        available_width
            .map(AvailableSpace::Definite)
            .unwrap_or(AvailableSpace::MaxContent)
    } else if let Some(available) = available_width {
        compute(&mut taffy, AvailableSpace::MaxContent, &mut paragraphs)?;
        let max_width = taffy.layout(root_node).ok()?.size.width;
        compute(&mut taffy, AvailableSpace::MinContent, &mut paragraphs)?;
        let min_width = taffy.layout(root_node).ok()?.size.width;
        AvailableSpace::Definite(available.max(min_width).min(max_width))
    } else {
        AvailableSpace::MaxContent
    };
    compute(&mut taffy, width_space, &mut paragraphs)?;

    let root_layout = taffy.layout(root_node).ok()?;
    let size = Size {
        width: root_layout.size.width,
        height: root_layout.size.height,
    };

    let mut positions: HashMap<NodeId, (f32, f32)> = HashMap::new();
    collect_positions(&taffy, root_node, 0.0, 0.0, &mut positions)?;

    let mut boxes: HashMap<LayoutBoxId, IsolatedBox> = HashMap::with_capacity(builder.nodes.len());
    for (box_id, node) in &builder.nodes {
        let Some(box_data) = tree.get(*box_id) else {
            continue;
        };
        let Some(layout) = taffy.layout(*node).ok() else {
            continue;
        };
        let Some(&(abs_x, abs_y)) = positions.get(node) else {
            continue;
        };
        let padding = &box_data.dimensions.padding;
        let border = &box_data.dimensions.border;
        let content = Rect::new(
            abs_x + border.left + padding.left,
            abs_y + border.top + padding.top,
            (layout.size.width - border.left - border.right - padding.left - padding.right).max(0.0),
            (layout.size.height - border.top - border.bottom - padding.top - padding.bottom).max(0.0),
        );
        boxes.insert(
            *box_id,
            IsolatedBox {
                content,
                margin: EdgeSizes::new(
                    layout.margin.top,
                    layout.margin.right,
                    layout.margin.bottom,
                    layout.margin.left,
                ),
                ..IsolatedBox::default()
            },
        );
    }

    for (box_id, _) in &builder.nodes {
        let Some(box_data) = tree.get(*box_id) else {
            continue;
        };
        if !matches!(box_data.box_type, AnonymousBoxType::Text(_)) {
            continue;
        }
        let Some(entry) = boxes.get_mut(box_id) else {
            continue;
        };
        let paragraph = text_cache.paragraph(box_data, Some(entry.content.width), font_db, false);
        entry.text_layout = Some(paragraph.as_ref().clone());
    }

    let paragraph_blocks = builder.paragraph_blocks.clone();
    for block_id in paragraph_blocks {
        let Some(context) = tree.inline_context(block_id) else {
            continue;
        };
        if !supports_context(tree, context) {
            continue;
        }
        let Some(origin) = boxes.get(&block_id).map(|entry| entry.content) else {
            continue;
        };
        let Some(paragraph) = paragraphs.get_or_compute(
            tree,
            context,
            Some(origin.width),
            None,
            text_cache,
            font_db,
            depth,
        ) else {
            continue;
        };
        let offset = tree
            .get(block_id)
            .map_or(0.0, |box_data| content_align_offset(&box_data.style, origin.height, paragraph.height));
        let origin = Rect::new(origin.x, origin.y + offset, origin.width, origin.height);
        let distribution = compute_distribution(tree, context, &paragraph, origin);
        merge_distribution(&mut boxes, &distribution);
        merge_atomic_subtrees(tree, context, &mut boxes, text_cache, font_db, depth);
    }

    Some(IsolatedLayout {
        size,
        boxes: boxes.into_iter().collect(),
    })
}

fn collect_positions(
    taffy: &TaffyTree<LayoutBoxId>,
    node: NodeId,
    x: f32,
    y: f32,
    out: &mut HashMap<NodeId, (f32, f32)>,
) -> Option<()> {
    let layout = taffy.layout(node).ok()?;
    let abs_x = x + layout.location.x;
    let abs_y = y + layout.location.y;
    out.insert(node, (abs_x, abs_y));
    for child in taffy.children(node).ok()? {
        collect_positions(taffy, child, abs_x, abs_y, out)?;
    }
    Some(())
}

fn merge_distribution(
    boxes: &mut HashMap<LayoutBoxId, IsolatedBox>,
    distribution: &ParagraphDistribution,
) {
    for entry in &distribution.direct {
        let target = boxes.entry(entry.id).or_default();
        target.content = entry.content;
        target.inline_fragments = entry.fragments.clone();
    }
    for entry in &distribution.text_boxes {
        let target = boxes.entry(entry.id).or_default();
        target.content = entry.content;
        target.inline_fragments = entry.fragments.clone();
        target.text_layout = Some(entry.layout.clone());
    }
    for entry in &distribution.objects {
        let target = boxes.entry(entry.id).or_default();
        target.content = entry.content;
        target.margin = entry.margin;
        target.inline_fragments = vec![entry.content];
    }
    for entry in &distribution.inline_owners {
        let target = boxes.entry(entry.id).or_default();
        target.content = entry.content;
        target.inline_fragments = entry.fragments.clone();
    }
}

fn merge_atomic_subtrees(
    tree: &LayoutTree,
    context: &crate::inline::InlineFormattingContext,
    boxes: &mut HashMap<LayoutBoxId, IsolatedBox>,
    text_cache: &mut TextLayoutCache,
    font_db: &FontDatabase,
    depth: u32,
) {
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
        let Some(rect) = boxes.get(id).map(|entry| entry.content) else {
            continue;
        };
        if let Some(nested) =
            layout_isolated(tree, *id, Some(rect.width), None, text_cache, font_db, depth + 1)
        {
            for (box_id, entry) in nested.boxes {
                boxes.insert(
                    box_id,
                    IsolatedBox {
                        content: Rect::new(
                            entry.content.x + rect.x,
                            entry.content.y + rect.y,
                            entry.content.width,
                            entry.content.height,
                        ),
                        margin: entry.margin,
                        text_layout: entry.text_layout,
                        inline_fragments: entry
                            .inline_fragments
                            .into_iter()
                            .map(|fragment| {
                                Rect::new(
                                    fragment.x + rect.x,
                                    fragment.y + rect.y,
                                    fragment.width,
                                    fragment.height,
                                )
                            })
                            .collect(),
                    },
                );
            }
        }
    }
}

pub(crate) fn apply_isolated_layout(
    tree: &mut LayoutTree,
    layout: &IsolatedLayout,
    offset_x: f32,
    offset_y: f32,
) {
    for (box_id, entry) in &layout.boxes {
        let Some(box_data) = tree.get_mut(*box_id) else {
            continue;
        };
        box_data.dimensions.content = Rect::new(
            entry.content.x + offset_x,
            entry.content.y + offset_y,
            entry.content.width,
            entry.content.height,
        );
        box_data.dimensions.margin = entry.margin;
        if !entry.inline_fragments.is_empty() {
            box_data.inline_fragments = entry
                .inline_fragments
                .iter()
                .map(|fragment| {
                    Rect::new(
                        fragment.x + offset_x,
                        fragment.y + offset_y,
                        fragment.width,
                        fragment.height,
                    )
                })
                .collect();
        }
        if let Some(text_layout) = &entry.text_layout {
            box_data.text_layout = Some(text_layout.clone());
        }
    }
}
