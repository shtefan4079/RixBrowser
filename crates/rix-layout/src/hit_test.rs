
use crate::overflow::viewport_overflow_owner;
use crate::tree::{LayoutBoxId, LayoutTree};
use rix_dom::{Document, NodeId};

pub fn hit_test_tree(tree: &LayoutTree, x: f32, y: f32) -> Option<NodeId> {
    hit_test_tree_with_scroll(tree, None, x, y, 0.0, 0.0)
}

pub fn hit_test_tree_with_scroll(
    tree: &LayoutTree,
    document: Option<&Document>,
    x: f32,
    y: f32,
    scroll_x: f32,
    scroll_y: f32,
) -> Option<NodeId> {
    let root_id = tree.root?;
    let viewport_overflow_owner = viewport_overflow_owner(tree, document);
    let mut best = None;
    hit_test_box_recursive(
        tree,
        root_id,
        x + scroll_x,
        y + scroll_y,
        x,
        y,
        false,
        viewport_overflow_owner,
        0,
        &mut 0,
        &mut best,
    );
    best.map(|candidate| candidate.node)
}

pub fn hit_test_box_with_scroll(
    tree: &LayoutTree,
    document: Option<&Document>,
    x: f32,
    y: f32,
    scroll_x: f32,
    scroll_y: f32,
) -> Option<LayoutBoxId> {
    let root_id = tree.root?;
    let viewport_overflow_owner = viewport_overflow_owner(tree, document);
    let mut best = None;
    hit_test_box_recursive(
        tree,
        root_id,
        x + scroll_x,
        y + scroll_y,
        x,
        y,
        false,
        viewport_overflow_owner,
        0,
        &mut 0,
        &mut best,
    );
    best.map(|candidate| candidate.box_id)
}

struct HitCandidate {
    node: NodeId,
    box_id: LayoutBoxId,
    z_index: i32,
    order: usize,
}

fn hit_test_box_recursive(
    tree: &LayoutTree,
    box_id: LayoutBoxId,
    page_x: f32,
    page_y: f32,
    viewport_x: f32,
    viewport_y: f32,
    inside_fixed: bool,
    viewport_overflow_owner: Option<NodeId>,
    accumulated_z: i32,
    order: &mut usize,
    best: &mut Option<HitCandidate>,
) {
    let Some(box_data) = tree.get(box_id) else {
        return;
    };
    let fixed = inside_fixed || box_data.style.position == "fixed";
    let (x, y) = if fixed {
        (viewport_x, viewport_y)
    } else {
        (page_x, page_y)
    };
    let z_index = accumulated_z.saturating_add(box_data.style.z_index);
    let visit_order = *order;
    *order += 1;
    let border_box = box_data.dimensions.border_box();

    let propagates_to_viewport =
        viewport_overflow_owner.is_some() && box_data.dom_node == viewport_overflow_owner;
    let clips_x = !propagates_to_viewport && box_data.style.overflow_x != "visible";
    let clips_y = !propagates_to_viewport && box_data.style.overflow_y != "visible";
    let padding = box_data.dimensions.padding_box();
    let descendants_clipped = (clips_x && (x < padding.x || x > padding.x + padding.width))
        || (clips_y && (y < padding.y || y > padding.y + padding.height));

    if !descendants_clipped {
        for child_id in &box_data.children {
            hit_test_box_recursive(
                tree,
                *child_id,
                page_x,
                page_y,
                viewport_x,
                viewport_y,
                fixed,
                viewport_overflow_owner,
                z_index,
                order,
                best,
            );
        }
    }

    let contains_point = if box_data.inline_fragments.is_empty() {
        border_box.contains(x, y)
    } else {
        box_data
            .inline_fragments
            .iter()
            .any(|fragment| fragment.contains(x, y))
    };
    if !contains_point
        || box_data.style.visibility != "visible"
        || box_data.style.pointer_events == "none"
    {
        return;
    }

    let node = if matches!(
        box_data.box_type,
        crate::tree::AnonymousBoxType::Text(_) | crate::tree::AnonymousBoxType::AnonymousBlock
    ) {
        box_data
            .parent
            .and_then(|parent_id| tree.get(parent_id))
            .and_then(|parent_box| parent_box.dom_node)
    } else {
        box_data.dom_node
    };
    let Some(node) = node else {
        return;
    };

    let paints_on_top = best.as_ref().is_none_or(|current| {
        (z_index, visit_order) >= (current.z_index, current.order)
    });
    if paints_on_top {
        *best = Some(HitCandidate {
            node,
            box_id,
            z_index,
            order: visit_order,
        });
    }
}

pub fn hit_test_text_offset(
    layout: &rix_text::ParagraphLayout,
    text: &str,
    rel_x: f32,
    rel_y: f32,
) -> usize {
    if layout.lines.is_empty() || text.is_empty() {
        return 0;
    }
    let line = layout
        .lines
        .iter()
        .find(|l| rel_y >= l.y && rel_y <= l.y + l.height)
        .or_else(|| {
            if rel_y < layout.lines[0].y {
                layout.lines.first()
            } else {
                layout.lines.last()
            }
        });
    let Some(line) = line else {
        return text.chars().count();
    };

    if line.glyphs.is_empty() {
        return 0;
    }

    if let Some(first) = line.glyphs.first() {
        if rel_x <= first.x {
            let byte_pos = first.cluster as usize;
            return text[..byte_pos.min(text.len())].chars().count();
        }
    }
    if let Some(last) = line.glyphs.last() {
        if rel_x >= last.x + last.advance {
            let byte_pos = (last.cluster + 1) as usize;
            return text[..byte_pos.min(text.len())].chars().count();
        }
    }

    for glyph in &line.glyphs {
        let mid = glyph.x + glyph.advance / 2.0;
        if rel_x < mid {
            let byte_pos = glyph.cluster as usize;
            return text[..byte_pos.min(text.len())].chars().count();
        }
    }

    if let Some(last) = line.glyphs.last() {
        let byte_pos = (last.cluster + 1) as usize;
        return text[..byte_pos.min(text.len())].chars().count();
    }
    text.chars().count()
}

pub fn hit_test_input_cursor(
    tree: &LayoutTree,
    node: NodeId,
    click_x: f32,
    click_y: f32,
) -> Option<usize> {
    for b in tree.boxes() {
        if b.dom_node == Some(node) {
            if let crate::tree::AnonymousBoxType::Text(ref text) = b.box_type {
                let rel_x = click_x - b.dimensions.content.x;
                let rel_y = click_y - b.dimensions.content.y;
                if let Some(layout) = &b.text_layout {
                    return Some(hit_test_text_offset(layout, text, rel_x, rel_y));
                } else {
                    let est_col = (rel_x.max(0.0) / 8.0).round() as usize;
                    return Some(est_col.min(text.chars().count()));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rix_text::layout::{LayoutGlyph, LayoutLine, ParagraphLayout};

    #[test]
    fn test_hit_test_text_offset_empty() {
        let layout = ParagraphLayout {
            lines: Vec::new(),
            inline_objects: Vec::new(),
            width: 0.0,
            height: 0.0,
        };
        assert_eq!(hit_test_text_offset(&layout, "", 10.0, 5.0), 0);
    }

    #[test]
    fn test_hit_test_text_offset_glyphs() {
        let text = "abc";
        let glyphs = vec![
            LayoutGlyph {
                glyph_id: 1,
                x: 0.0,
                advance: 10.0,
                y: 0.0,
                cluster: 0,
                source: None,
                paint: true,
                font_id: rix_text::FontId(0),
                font_size: 14.0,
                color: [0.0; 4],
            },
            LayoutGlyph {
                glyph_id: 2,
                x: 10.0,
                advance: 10.0,
                y: 0.0,
                cluster: 1,
                source: None,
                paint: true,
                font_id: rix_text::FontId(0),
                font_size: 14.0,
                color: [0.0; 4],
            },
            LayoutGlyph {
                glyph_id: 3,
                x: 20.0,
                advance: 10.0,
                y: 0.0,
                cluster: 2,
                source: None,
                paint: true,
                font_id: rix_text::FontId(0),
                font_size: 14.0,
                color: [0.0; 4],
            },
        ];
        let layout = ParagraphLayout {
            lines: vec![LayoutLine {
                glyphs,
                y: 0.0,
                width: 30.0,
                height: 18.0,
                ascent: 14.0,
                descent: 4.0,
                baseline: 14.0,
            }],
            inline_objects: Vec::new(),
            width: 30.0,
            height: 18.0,
        };

        assert_eq!(hit_test_text_offset(&layout, text, -5.0, 5.0), 0);
        assert_eq!(hit_test_text_offset(&layout, text, 2.0, 5.0), 0);
        
        assert_eq!(hit_test_text_offset(&layout, text, 12.0, 5.0), 1);
        
        assert_eq!(hit_test_text_offset(&layout, text, 18.0, 5.0), 2);
        
        assert_eq!(hit_test_text_offset(&layout, text, 35.0, 5.0), 3);
    }
}
