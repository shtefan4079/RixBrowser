
use crate::box_model::{EdgeSizes, Rect};
use crate::inline::{InlineFlowItem, InlineFormattingContext};
use crate::tree::{LayoutBoxId, LayoutTree};
use rix_css::computed::{ComputedStyle, Display};
use rix_text::layout::{line_metrics_for_font, LayoutGlyph};
use rix_text::{
    contains_rtl_runs, FontDatabase, FontDescriptor, FontId, FontStyle, FontWeight,
    InlineObjectMetrics, LayoutLine, ParagraphItem, ParagraphLayout, ParagraphOptions,
    SpaceBreakBehavior, TextAlign, TextSpan, TrailingSpaceBehavior,
};
use std::collections::HashMap;

pub(crate) fn supports_context(tree: &LayoutTree, context: &InlineFormattingContext) -> bool {
    let Some(block) = tree.get(context.block) else {
        return false;
    };
    let has_atomic = context
        .items
        .iter()
        .any(|item| matches!(item, InlineFlowItem::Atomic(_)));
    if (context.runs.is_empty() && !has_atomic)
        || (block.style.text_overflow == "ellipsis" && block.style.overflow_x != "visible")
    {
        return false;
    }

    let white_space = if block.style.white_space.as_str() != "normal" {
        block.style.white_space.as_str()
    } else {
        context
            .runs
            .first()
            .map(|run| run.style.white_space.as_str())
            .unwrap_or("normal")
    };
    if context
        .runs
        .iter()
        .any(|run| !compatible_white_space_modes(white_space, &run.style.white_space))
    {
        return false;
    }
    if context
        .items
        .iter()
        .any(|item| matches!(item, InlineFlowItem::Atomic(_)))
        && contains_rtl_runs(&context.text)
    {
        return false;
    }

    context.items.iter().all(|item| match item {
        InlineFlowItem::TextRun(_) | InlineFlowItem::InlineEnd(_) => true,
        InlineFlowItem::InlineStart(id) | InlineFlowItem::ForcedBreak(id) => {
            inline_box_supported(tree, *id)
        }
        InlineFlowItem::Atomic(id) => atomic_box_supported(tree, *id),
        InlineFlowItem::BlockBoundary(_) => false,
    })
}

fn compatible_white_space_modes(first: &str, next: &str) -> bool {
    first == next
        || ["normal", "pre-line", "pre-wrap"].contains(&first)
            && ["normal", "pre-line", "pre-wrap"].contains(&next)
}

fn atomic_box_supported(tree: &LayoutTree, id: LayoutBoxId) -> bool {
    let Some(box_data) = tree.get(id) else {
        return false;
    };
    let style = &box_data.style;
    (box_data.is_replaced || style.display.is_atomic_inline())
        && matches!(style.position.as_str(), "static" | "relative")
        && style.padding_left == 0.0
        && style.padding_right == 0.0
        && style.padding_top == 0.0
        && style.padding_bottom == 0.0
        && style.border_top_width == 0.0
        && style.border_right_width == 0.0
        && style.border_bottom_width == 0.0
        && style.border_left_width == 0.0
}

fn inline_box_supported(tree: &LayoutTree, id: LayoutBoxId) -> bool {
    let Some(box_data) = tree.get(id) else {
        return false;
    };
    let style = &box_data.style;
    style.display == Display::Inline
        && style.position == "static"
        && style.margin_left == 0.0
        && style.margin_right == 0.0
        && style.padding_left == 0.0
        && style.padding_right == 0.0
        && style.padding_top == 0.0
        && style.padding_bottom == 0.0
        && style.border_top_width == 0.0
        && style.border_right_width == 0.0
        && style.border_bottom_width == 0.0
        && style.border_left_width == 0.0
        && style.border_radius == 0.0
        && style.box_shadow.is_none()
        && style.background_image.is_none()
}

fn block_font_id(block_style: &ComputedStyle, font_db: &FontDatabase) -> Option<FontId> {
    let descriptor = FontDescriptor::new(&block_style.font_family)
        .with_weight(FontWeight(block_style.font_weight))
        .with_style(match block_style.font_style.as_str() {
            "italic" => FontStyle::Italic,
            "oblique" => FontStyle::Oblique,
            _ => FontStyle::Normal,
        });
    font_db.match_font(&descriptor)
}

fn block_font_metrics(block_style: &ComputedStyle, font_db: &FontDatabase) -> Option<(f32, f32, f32)> {
    let font = font_db.get_font(block_font_id(block_style, font_db)?)?;
    let metrics = font.metrics;
    Some((
        metrics.ascender_px(block_style.font_size),
        metrics.descender_px(block_style.font_size).abs(),
        metrics
            .x_height_px(block_style.font_size)
            .unwrap_or(block_style.font_size * 0.5),
    ))
}

fn vertical_align_shift(value: &str, font_size: f32, line_height: f32) -> Option<f32> {
    let v = value.trim();
    if let Some(percent) = v.strip_suffix('%') {
        return percent
            .trim()
            .parse::<f32>()
            .ok()
            .map(|p| p / 100.0 * line_height);
    }
    for (unit, scale) in [("px", 1.0), ("rem", 16.0), ("em", font_size)] {
        if let Some(number) = v.strip_suffix(unit) {
            return number.trim().parse::<f32>().ok().map(|n| n * scale);
        }
    }
    None
}

fn atomic_baseline(
    alignment: &str,
    height: f32,
    block_style: &ComputedStyle,
    metrics: Option<(f32, f32, f32)>,
    natural_baseline: f32,
) -> f32 {
    let (ascent, descent, x_height) = metrics.unwrap_or((
        block_style.font_size * 0.8,
        block_style.font_size * 0.2,
        block_style.font_size * 0.5,
    ));
    match alignment {
        "middle" => (height - x_height) / 2.0,
        "text-top" => ascent,
        "text-bottom" => height - descent,
        "sub" => height - block_style.font_size * 0.2,
        "super" => height + block_style.font_size * 0.2,
        "baseline" => natural_baseline,
        other => vertical_align_shift(other, block_style.font_size, block_style.line_height)
            .map_or(natural_baseline, |shift| natural_baseline + shift),
    }
}

pub(crate) fn content_align_offset(
    block_style: &ComputedStyle,
    content_height: f32,
    paragraph_height: f32,
) -> f32 {
    if block_style.align_content != "center" {
        return 0.0;
    }
    let slack = content_height - paragraph_height;
    if slack > 0.0 {
        slack / 2.0
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct AtomicSize {
    pub(crate) border_width: f32,
    pub(crate) border_height: f32,
    pub(crate) margin: EdgeSizes,
    pub(crate) baseline: Option<f32>,
}

impl AtomicSize {
    pub(crate) fn margin_box(&self) -> (f32, f32) {
        (
            self.border_width + self.margin.left + self.margin.right,
            self.border_height + self.margin.top + self.margin.bottom,
        )
    }
}

pub(crate) fn compute_paragraph(
    context: &InlineFormattingContext,
    block_style: &ComputedStyle,
    max_width: Option<f32>,
    font_db: &FontDatabase,
    atomic_sizes: &HashMap<LayoutBoxId, AtomicSize>,
    atomic_alignments: &HashMap<LayoutBoxId, String>,
) -> Option<ParagraphLayout> {
    let mut spans = Vec::with_capacity(context.runs.len());
    for run in &context.runs {
        let descriptor = FontDescriptor::new(&run.style.font_family)
            .with_weight(FontWeight(run.style.font_weight))
            .with_style(match run.style.font_style.as_str() {
                "italic" => FontStyle::Italic,
                "oblique" => FontStyle::Oblique,
                _ => FontStyle::Normal,
            });
        let font_id = font_db.match_font(&descriptor)?;
        let text = context.text.get(run.range.clone())?;
        let color = run.style.color;
        let mut span = TextSpan::new(text, font_id, run.style.font_size)
            .with_color([color.r, color.g, color.b, color.a])
            .with_trailing_space_behavior(match run.style.white_space.as_str() {
                "pre-wrap" => TrailingSpaceBehavior::Hang,
                "pre" | "break-spaces" => TrailingSpaceBehavior::Preserve,
                _ => TrailingSpaceBehavior::Remove,
            });
        if !run.style.line_height_normal {
            span = span.with_line_height(run.style.line_height);
        }
        spans.push(span);
    }

    let white_space = if block_style.white_space.as_str() != "normal" {
        block_style.white_space.as_str()
    } else {
        context
            .runs
            .first()
            .map(|run| run.style.white_space.as_str())
            .unwrap_or("normal")
    };
    let wrap = !matches!(white_space, "nowrap" | "pre");
    let trailing_space_behavior = match white_space {
        "pre-wrap" => TrailingSpaceBehavior::Hang,
        "pre" | "break-spaces" => TrailingSpaceBehavior::Preserve,
        _ => TrailingSpaceBehavior::Remove,
    };
    let space_break_behavior = if white_space == "break-spaces" {
        SpaceBreakBehavior::AfterEachPreservedSpace
    } else {
        SpaceBreakBehavior::Unicode
    };
    let align = match block_style.text_align.as_str() {
        "center" => TextAlign::Center,
        "right" | "end" => TextAlign::Right,
        "justify" => TextAlign::Justify,
        _ => TextAlign::Left,
    };
    let forced_break_runs: HashMap<_, _> = context
        .runs
        .iter()
        .enumerate()
        .filter(|(_, run)| run.forced_break)
        .filter_map(|(index, run)| run.source_box.map(|box_id| (box_id, index)))
        .collect();
    let mut content = Vec::with_capacity(context.items.len());
    let needs_metrics = atomic_alignments
        .values()
        .any(|alignment| alignment != "baseline");
    let parent_metrics = if needs_metrics {
        block_font_metrics(block_style, font_db)
    } else {
        None
    };
    for item in &context.items {
        match item {
            InlineFlowItem::TextRun(index) => content.push(ParagraphItem::TextSpan(*index)),
            InlineFlowItem::ForcedBreak(id) => {
                let index = forced_break_runs.get(id).copied()?;
                content.push(ParagraphItem::TextSpan(index));
            }
            InlineFlowItem::Atomic(id) => {
                let size = atomic_sizes.get(id).copied()?;
                let (width, height) = size.margin_box();
                let alignment = atomic_alignments
                    .get(id)
                    .map(String::as_str)
                    .unwrap_or("baseline");
                let natural_baseline = size
                    .baseline
                    .map(|b| b + size.margin.top)
                    .unwrap_or(height);
                content.push(ParagraphItem::InlineObject(InlineObjectMetrics {
                    id: id.0,
                    width,
                    height,
                    baseline: atomic_baseline(
                        alignment,
                        height,
                        block_style,
                        parent_metrics,
                        natural_baseline,
                    ),
                }));
            }
            InlineFlowItem::InlineStart(_)
            | InlineFlowItem::InlineEnd(_)
            | InlineFlowItem::BlockBoundary(_) => {}
        }
    }

    let strut = block_font_id(block_style, font_db).map(|font_id| {
        line_metrics_for_font(
            font_db,
            font_id,
            block_style.font_size,
            (!block_style.line_height_normal).then_some(block_style.line_height),
        )
    });

    Some(ParagraphLayout::compute_with_content(
        &spans,
        &content,
        align,
        max_width,
        font_db,
        ParagraphOptions {
            wrap,
            trailing_space_behavior,
            space_break_behavior,
            strut,
        },
    ))
}

pub(crate) struct DistributedTextBox {
    pub(crate) id: LayoutBoxId,
    pub(crate) content: Rect,
    pub(crate) fragments: Vec<Rect>,
    pub(crate) layout: ParagraphLayout,
}

pub(crate) struct DistributedBoxGeometry {
    pub(crate) id: LayoutBoxId,
    pub(crate) content: Rect,
    pub(crate) fragments: Vec<Rect>,
}

pub(crate) struct DistributedInlineOwner {
    pub(crate) id: LayoutBoxId,
    pub(crate) content: Rect,
    pub(crate) fragments: Vec<Rect>,
}

pub(crate) struct DistributedObject {
    pub(crate) id: LayoutBoxId,
    pub(crate) content: Rect,
    pub(crate) margin: EdgeSizes,
}

#[derive(Default)]
pub(crate) struct ParagraphDistribution {
    pub(crate) direct: Vec<DistributedBoxGeometry>,
    pub(crate) text_boxes: Vec<DistributedTextBox>,
    pub(crate) objects: Vec<DistributedObject>,
    pub(crate) inline_owners: Vec<DistributedInlineOwner>,
}

pub(crate) fn compute_distribution(
    tree: &LayoutTree,
    context: &InlineFormattingContext,
    paragraph: &ParagraphLayout,
    origin: Rect,
) -> ParagraphDistribution {
    let mut distribution = ParagraphDistribution::default();

    let mut owners_by_run = vec![Vec::new(); context.runs.len()];
    let forced_break_runs: HashMap<_, _> = context
        .runs
        .iter()
        .enumerate()
        .filter(|(_, run)| run.forced_break)
        .filter_map(|(index, run)| run.source_box.map(|box_id| (box_id, index)))
        .collect();
    let mut stack: Vec<LayoutBoxId> = Vec::new();
    let mut owners_by_atomic: HashMap<LayoutBoxId, Vec<LayoutBoxId>> = HashMap::new();
    for item in &context.items {
        match item {
            InlineFlowItem::InlineStart(id) => stack.push(*id),
            InlineFlowItem::InlineEnd(id) => {
                if stack.last() == Some(id) {
                    stack.pop();
                } else if let Some(index) = stack.iter().rposition(|candidate| candidate == id) {
                    stack.truncate(index);
                }
            }
            InlineFlowItem::TextRun(index) => {
                if let Some(owners) = owners_by_run.get_mut(*index) {
                    *owners = stack.clone();
                }
            }
            InlineFlowItem::ForcedBreak(id) => {
                let run_index = forced_break_runs.get(id).copied();
                if let Some(owners) = run_index.and_then(|index| owners_by_run.get_mut(index)) {
                    *owners = stack.clone();
                }
            }
            InlineFlowItem::Atomic(id) => {
                owners_by_atomic.insert(*id, stack.clone());
            }
            InlineFlowItem::BlockBoundary(_) => {}
        }
    }

    let mut text_lines: HashMap<LayoutBoxId, Vec<(LayoutLine, Rect, Vec<LayoutGlyph>)>> =
        HashMap::new();
    let mut inline_lines: HashMap<LayoutBoxId, Vec<Option<Rect>>> = HashMap::new();

    for (line_index, line) in paragraph.lines.iter().enumerate() {
        let mut glyphs_by_run: HashMap<usize, Vec<LayoutGlyph>> = HashMap::new();
        for glyph in &line.glyphs {
            let Some(source) = glyph.source else {
                continue;
            };
            glyphs_by_run
                .entry(source.span_index)
                .or_default()
                .push(*glyph);
        }

        for (run_index, glyphs) in glyphs_by_run {
            let Some(run) = context.runs.get(run_index) else {
                continue;
            };
            let Some(bounds) = glyph_bounds(line, &glyphs) else {
                continue;
            };
            for owner in owners_by_run.get(run_index).into_iter().flatten() {
                let fragments = inline_lines
                    .entry(*owner)
                    .or_insert_with(|| vec![None; paragraph.lines.len()]);
                if let Some(fragment) = fragments.get_mut(line_index) {
                    *fragment =
                        Some(fragment.map_or(bounds, |current| union_rect(current, bounds)));
                }
            }

            if let Some(source_box) = run.source_box {
                text_lines
                    .entry(source_box)
                    .or_default()
                    .push((line.clone(), bounds, glyphs));
            }
        }
    }

    let mut next_break_line = 0usize;
    for (run_index, run) in context
        .runs
        .iter()
        .enumerate()
        .filter(|(_, run)| run.forced_break)
    {
        let previous_text_line = paragraph
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                line.glyphs.iter().any(|glyph| {
                    glyph
                        .source
                        .is_some_and(|source| source.span_index < run_index)
                })
            })
            .map(|(line_index, _)| line_index)
            .max();
        let line_index = previous_text_line
            .unwrap_or(next_break_line)
            .max(next_break_line);
        let Some(line) = paragraph.lines.get(line_index) else {
            continue;
        };
        let x = line
            .glyphs
            .iter()
            .filter(|glyph| {
                glyph
                    .source
                    .is_some_and(|source| source.span_index < run_index)
            })
            .map(|glyph| glyph.x + glyph.advance)
            .fold(0.0, f32::max);
        let local_fragment = Rect::new(x, line.y, 0.0, line.height);
        if let Some(box_id) = run.source_box {
            let fragment = Rect::new(
                origin.x + local_fragment.x,
                origin.y + local_fragment.y,
                local_fragment.width,
                local_fragment.height,
            );
            distribution.direct.push(DistributedBoxGeometry {
                id: box_id,
                content: fragment,
                fragments: vec![fragment],
            });
        }
        for owner in owners_by_run.get(run_index).into_iter().flatten() {
            let fragments = inline_lines
                .entry(*owner)
                .or_insert_with(|| vec![None; paragraph.lines.len()]);
            if let Some(fragment) = fragments.get_mut(line_index) {
                *fragment = Some(fragment.map_or(local_fragment, |current| {
                    union_rect(current, local_fragment)
                }));
            }
        }
        next_break_line = line_index.saturating_add(1);
    }

    for (box_id, mut lines) in text_lines {
        lines.sort_by(|left, right| left.0.y.total_cmp(&right.0.y));
        let min_x = lines
            .iter()
            .map(|(_, bounds, _)| bounds.x)
            .fold(f32::INFINITY, f32::min);
        let max_x = lines
            .iter()
            .map(|(_, bounds, _)| bounds.x + bounds.width)
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = lines
            .iter()
            .map(|(_, bounds, _)| bounds.y)
            .fold(f32::INFINITY, f32::min);
        let max_y = lines
            .iter()
            .map(|(_, bounds, _)| bounds.y + bounds.height)
            .fold(f32::NEG_INFINITY, f32::max);
        if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
            continue;
        }

        let mut layout_lines = Vec::with_capacity(lines.len());
        let source_fragments: Vec<_> = lines
            .iter()
            .map(|(_, bounds, _)| {
                Rect::new(
                    origin.x + bounds.x,
                    origin.y + bounds.y,
                    bounds.width,
                    bounds.height,
                )
            })
            .collect();
        for (mut line, bounds, mut glyphs) in lines {
            for glyph in &mut glyphs {
                glyph.x -= min_x;
                glyph.y -= min_y;
            }
            line.glyphs = glyphs;
            line.y -= min_y;
            line.baseline -= min_y;
            line.width = bounds.width;
            layout_lines.push(line);
        }
        let width = (max_x - min_x).max(0.0);
        let height = (max_y - min_y).max(0.0);
        distribution.text_boxes.push(DistributedTextBox {
            id: box_id,
            content: Rect::new(origin.x + min_x, origin.y + min_y, width, height),
            fragments: source_fragments,
            layout: ParagraphLayout {
                lines: layout_lines,
                inline_objects: Vec::new(),
                width,
                height,
            },
        });
    }

    for object in &paragraph.inline_objects {
        let box_id = LayoutBoxId(object.id);
        let margin = tree
            .get(box_id)
            .map(|box_data| {
                EdgeSizes::new(
                    box_data.style.margin_top,
                    box_data.style.margin_right,
                    box_data.style.margin_bottom,
                    box_data.style.margin_left,
                )
            })
            .unwrap_or(EdgeSizes::ZERO);
        let local_fragment = Rect::new(
            object.x + margin.left,
            object.y + margin.top,
            (object.width - margin.left - margin.right).max(0.0),
            (object.height - margin.top - margin.bottom).max(0.0),
        );
        let fragment = Rect::new(
            origin.x + local_fragment.x,
            origin.y + local_fragment.y,
            local_fragment.width,
            local_fragment.height,
        );
        distribution.objects.push(DistributedObject {
            id: box_id,
            content: fragment,
            margin,
        });
        for owner in owners_by_atomic.get(&box_id).into_iter().flatten() {
            let fragments = inline_lines
                .entry(*owner)
                .or_insert_with(|| vec![None; paragraph.lines.len()]);
            if let Some(fragment) = fragments.get_mut(object.line_index) {
                *fragment = Some(fragment.map_or(local_fragment, |current| {
                    union_rect(current, local_fragment)
                }));
            }
        }
    }

    for (box_id, fragments) in inline_lines {
        let fragments: Vec<_> = fragments
            .into_iter()
            .flatten()
            .map(|fragment| {
                Rect::new(
                    origin.x + fragment.x,
                    origin.y + fragment.y,
                    fragment.width,
                    fragment.height,
                )
            })
            .collect();
        if fragments.is_empty() {
            continue;
        }
        let bounds = fragments
            .iter()
            .copied()
            .reduce(union_rect)
            .unwrap_or_default();
        distribution.inline_owners.push(DistributedInlineOwner {
            id: box_id,
            content: bounds,
            fragments,
        });
    }

    distribution
}

pub(crate) fn apply_distribution(tree: &mut LayoutTree, distribution: &ParagraphDistribution) {
    for entry in &distribution.direct {
        if let Some(box_data) = tree.get_mut(entry.id) {
            box_data.dimensions.content = entry.content;
            box_data.inline_fragments = entry.fragments.clone();
        }
    }
    for entry in &distribution.text_boxes {
        if let Some(box_data) = tree.get_mut(entry.id) {
            box_data.dimensions.content = entry.content;
            box_data.inline_fragments = entry.fragments.clone();
            box_data.text_layout = Some(entry.layout.clone());
        }
    }
    for entry in &distribution.objects {
        if let Some(box_data) = tree.get_mut(entry.id) {
            box_data.dimensions.content = entry.content;
            box_data.dimensions.margin = entry.margin;
            box_data.inline_fragments = vec![entry.content];
        }
    }
    for entry in &distribution.inline_owners {
        if let Some(box_data) = tree.get_mut(entry.id) {
            box_data.inline_fragments = entry.fragments.clone();
            box_data.dimensions.content = entry.content;
        }
    }
}

fn glyph_bounds(line: &LayoutLine, glyphs: &[LayoutGlyph]) -> Option<Rect> {
    let min_x = glyphs
        .iter()
        .map(|glyph| glyph.x.min(glyph.x + glyph.advance))
        .fold(f32::INFINITY, f32::min);
    let max_x = glyphs
        .iter()
        .map(|glyph| glyph.x.max(glyph.x + glyph.advance))
        .fold(f32::NEG_INFINITY, f32::max);
    (min_x.is_finite() && max_x.is_finite())
        .then(|| Rect::new(min_x, line.y, (max_x - min_x).max(0.0), line.height))
}

fn union_rect(left: Rect, right: Rect) -> Rect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (left.x + left.width).max(right.x + right.width);
    let bottom = (left.y + left.height).max(right.y + right.height);
    Rect::new(x, y, right_edge - x, bottom - y)
}
