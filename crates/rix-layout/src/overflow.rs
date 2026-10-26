
use crate::box_model::Rect;
use crate::tree::LayoutTree;
use rix_dom::{Document, NodeData};

pub fn viewport_overflow_owner(
    tree: &LayoutTree,
    document: Option<&Document>,
) -> Option<rix_dom::NodeId> {
    let document = document?;
    let root_element = document
        .children(document.root())
        .find(|node| matches!(document.get(*node), Some(NodeData::Element(_))))?;
    let root_visible = tree
        .iter()
        .find(|item| item.dom_node == Some(root_element))
        .is_none_or(|item| {
            item.style.overflow_x == "visible" && item.style.overflow_y == "visible"
        });
    if !root_visible {
        return Some(root_element);
    }
    document.children(root_element).find(|node| {
        matches!(
            document.get(*node),
            Some(NodeData::Element(element)) if &*element.name.local == "body"
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportScrollRange {
    pub min_x: f32,
    pub max_x: f32,
    pub min_y: f32,
    pub max_y: f32,
}

impl ViewportScrollRange {
    pub fn clamp_x(self, value: f32) -> f32 {
        clamp_axis(value, self.min_x, self.max_x)
    }

    pub fn clamp_y(self, value: f32) -> f32 {
        clamp_axis(value, self.min_y, self.max_y)
    }
}

fn clamp_axis(value: f32, min: f32, max: f32) -> f32 {
    if value.is_nan() {
        min
    } else {
        value.clamp(min, max)
    }
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn finite_rect(rect: Rect) -> Option<Rect> {
    (rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width >= 0.0
        && rect.height >= 0.0)
        .then_some(rect)
}

fn clip_axis(start: &mut f32, size: &mut f32, clip_start: f32, clip_size: f32) -> bool {
    let end = *start + *size;
    let clip_end = clip_start + clip_size;
    let clipped_start = (*start).max(clip_start);
    let clipped_end = end.min(clip_end);
    if clipped_start >= clipped_end {
        *start = clipped_start;
        *size = 0.0;
        return false;
    }
    *start = clipped_start;
    *size = clipped_end - clipped_start;
    true
}

fn has_fixed_ancestor(tree: &LayoutTree, mut ancestor: Option<crate::tree::LayoutBoxId>) -> bool {
    while let Some(id) = ancestor {
        let Some(item) = tree.get(id) else {
            return true;
        };
        if item.style.position == "fixed" {
            return true;
        }
        ancestor = item.parent;
    }
    false
}

fn clipping_ancestors_for(
    tree: &LayoutTree,
    item: &crate::tree::LayoutBoxData,
) -> Option<crate::tree::LayoutBoxId> {
    if item.style.position != "absolute" {
        return item.parent;
    }

    let mut ancestor = item.parent;
    while let Some(id) = ancestor {
        let parent = tree.get(id)?;
        if parent.style.position != "static" {
            return Some(id);
        }
        ancestor = parent.parent;
    }
    None
}

pub fn viewport_scroll_range(tree: &LayoutTree, viewport: (f32, f32)) -> ViewportScrollRange {
    let viewport_width = finite_nonnegative(viewport.0);
    let viewport_height = finite_nonnegative(viewport.1);
    let mut right = viewport_width;
    let mut bottom = viewport_height;

    for item in tree.iter() {
        if item.style.position == "fixed" || has_fixed_ancestor(tree, item.parent) {
            continue;
        }
        let Some(mut margin_box) = finite_rect(item.dimensions.margin_box()) else {
            continue;
        };

        let mut ancestor = clipping_ancestors_for(tree, item);
        let mut valid = true;
        let mut contributes_x = true;
        let mut contributes_y = true;
        while let Some(id) = ancestor {
            let Some(parent) = tree.get(id) else {
                valid = false;
                break;
            };
            let Some(clip) = finite_rect(parent.dimensions.padding_box()) else {
                valid = false;
                break;
            };
            if parent.style.overflow_x != "visible" {
                contributes_x &=
                    clip_axis(&mut margin_box.x, &mut margin_box.width, clip.x, clip.width);
            }
            if parent.style.overflow_y != "visible" {
                contributes_y &= clip_axis(
                    &mut margin_box.y,
                    &mut margin_box.height,
                    clip.y,
                    clip.height,
                );
            }
            ancestor = parent.parent;
        }
        if !valid {
            continue;
        }

        let candidate_right = margin_box.x + margin_box.width;
        let candidate_bottom = margin_box.y + margin_box.height;
        if contributes_x && candidate_right.is_finite() {
            right = right.max(candidate_right);
        }
        if contributes_y && candidate_bottom.is_finite() {
            bottom = bottom.max(candidate_bottom);
        }
    }

    ViewportScrollRange {
        min_x: 0.0,
        max_x: (right - viewport_width).max(0.0),
        min_y: 0.0,
        max_y: (bottom - viewport_height).max(0.0),
    }
}
