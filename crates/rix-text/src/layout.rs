
use crate::font_loader::{FontDatabase, FontId};
use crate::shaping::TextShaper;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    pub text: String,
    pub font_id: FontId,
    pub font_size: f32,
    pub color: [f32; 4],
    pub line_height: Option<f32>,
    pub trailing_space_behavior: Option<TrailingSpaceBehavior>,
}

impl TextSpan {
    pub fn new(text: impl Into<String>, font_id: FontId, font_size: f32) -> Self {
        Self {
            text: text.into(),
            font_id,
            font_size,
            color: [0.0, 0.0, 0.0, 1.0],
            line_height: None,
            trailing_space_behavior: None,
        }
    }

    pub fn with_color(mut self, color: [f32; 4]) -> Self {
        self.color = color;
        self
    }

    pub fn with_line_height(mut self, line_height: f32) -> Self {
        self.line_height = Some(line_height.max(0.0));
        self
    }

    pub fn with_trailing_space_behavior(mut self, behavior: TrailingSpaceBehavior) -> Self {
        self.trailing_space_behavior = Some(behavior);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrailingSpaceBehavior {
    #[default]
    Remove,
    Hang,
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpaceBreakBehavior {
    #[default]
    Unicode,
    AfterEachPreservedSpace,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineStrut {
    pub ascent: f32,
    pub descent: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct ParagraphOptions {
    pub wrap: bool,
    pub trailing_space_behavior: TrailingSpaceBehavior,
    pub space_break_behavior: SpaceBreakBehavior,
    pub strut: Option<LineStrut>,
}

impl Default for ParagraphOptions {
    fn default() -> Self {
        Self {
            wrap: true,
            trailing_space_behavior: TrailingSpaceBehavior::Remove,
            space_break_behavior: SpaceBreakBehavior::Unicode,
            strut: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutGlyph {
    pub glyph_id: u32,
    pub x: f32,
    pub advance: f32,
    pub y: f32,
    pub cluster: u32,
    pub source: Option<TextSourceRange>,
    pub paint: bool,
    pub font_id: FontId,
    pub font_size: f32,
    pub color: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSourceRange {
    pub span_index: usize,
    pub byte_start: u32,
    pub byte_end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParagraphItem {
    TextSpan(usize),
    InlineObject(InlineObjectMetrics),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InlineObjectMetrics {
    pub id: usize,
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedInlineObject {
    pub id: usize,
    pub line_index: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutLine {
    pub glyphs: Vec<LayoutGlyph>,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub ascent: f32,
    pub descent: f32,
    pub baseline: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphLayout {
    pub lines: Vec<LayoutLine>,
    pub inline_objects: Vec<PositionedInlineObject>,
    pub width: f32,
    pub height: f32,
}

struct Segment {
    pieces: Vec<SegmentPiece>,
    metric_span_idx: usize,
    mandatory_after: bool,
    is_newline_break: bool,
}

enum SegmentPiece {
    Text(SegmentPart),
    InlineObject(InlineObjectMetrics),
}

struct SegmentPart {
    span_idx: usize,
    byte_range: std::ops::Range<usize>,
    shape_text: String,
}

struct ShapedPart {
    sources: Vec<ShapedSourcePart>,
    shape_text: String,
    runs: Vec<crate::shaping::ShapedRun>,
}

struct ShapedSourcePart {
    span_idx: usize,
    byte_range: std::ops::Range<usize>,
    text_range: std::ops::Range<usize>,
}

struct ShapedSegment {
    pieces: Vec<ShapedPiece>,
    width: f32,
    trailing_space_width: f32,
    has_non_space_content: bool,
    metric_span_idx: usize,
    mandatory_after: bool,
    is_newline_break: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEnd {
    SoftWrap,
    ForcedBreak,
    ParagraphEnd,
}

enum ShapedPiece {
    Text(ShapedPart),
    InlineObject(InlineObjectMetrics),
}

impl ParagraphLayout {
    pub fn compute(
        spans: &[TextSpan],
        align: TextAlign,
        max_width: Option<f32>,
        db: &FontDatabase,
    ) -> Self {
        Self::compute_with_options(spans, align, max_width, db, ParagraphOptions::default())
    }

    pub fn compute_with_options(
        spans: &[TextSpan],
        align: TextAlign,
        max_width: Option<f32>,
        db: &FontDatabase,
        options: ParagraphOptions,
    ) -> Self {
        let content: Vec<_> = (0..spans.len()).map(ParagraphItem::TextSpan).collect();
        Self::compute_with_content(spans, &content, align, max_width, db, options)
    }

    pub fn compute_with_content(
        spans: &[TextSpan],
        content: &[ParagraphItem],
        align: TextAlign,
        max_width: Option<f32>,
        db: &FontDatabase,
        options: ParagraphOptions,
    ) -> Self {
        if spans.is_empty() && content.is_empty() {
            return Self {
                lines: Vec::new(),
                inline_objects: Vec::new(),
                width: 0.0,
                height: 0.0,
            };
        }

        let shaper = TextShaper::new(db);

        let mut segments = split_content_uax14(spans, content, db, options.space_break_behavior);
        if !options.wrap {
            let mut joined: Vec<Segment> = Vec::new();
            for segment in segments {
                if let Some(previous) = joined
                    .last_mut()
                    .filter(|s| !s.mandatory_after && !s.is_newline_break)
                {
                    previous.pieces.extend(segment.pieces);
                    previous.metric_span_idx = segment.metric_span_idx;
                    previous.mandatory_after = segment.mandatory_after;
                } else {
                    joined.push(segment);
                }
            }
            segments = joined;
        }

        if segments.is_empty() {
            if let Some(span) = spans
                .iter()
                .find(|span| db.get_font(span.font_id).is_some())
            {
                let (ascent, descent, height) = line_metrics_for_span(db, span);
                return Self {
                    lines: vec![LayoutLine {
                        glyphs: Vec::new(),
                        y: 0.0,
                        width: 0.0,
                        height,
                        ascent,
                        descent,
                        baseline: ascent,
                    }],
                    inline_objects: Vec::new(),
                    width: 0.0,
                    height,
                };
            }
            return Self {
                lines: Vec::new(),
                inline_objects: Vec::new(),
                width: 0.0,
                height: 0.0,
            };
        }

        let mut lines: Vec<LayoutLine> = Vec::new();
        let mut current_y = 0.0;

        let mut line_items: Vec<LineItem> = Vec::new();
        let mut line_objects: Vec<LineObject> = Vec::new();
        let mut trailing_space_indices: Vec<usize> = Vec::new();
        let mut inline_objects = Vec::new();
        let mut line_width = 0.0;
        let mut line_ascent = 0.0f32;
        let mut line_descent = 0.0f32;
        let mut line_height = 0.0f32;

        let mut shaped_segments = Vec::with_capacity(segments.len());
        for seg in segments {
            let mut pieces = Vec::with_capacity(seg.pieces.len());
            let mut width = 0.0;
            let mut segment_pieces = seg.pieces.into_iter().peekable();
            while let Some(piece) = segment_pieces.next() {
                match piece {
                    SegmentPiece::Text(part) => {
                        let Some(span) = spans.get(part.span_idx) else {
                            continue;
                        };
                        if part.shape_text.is_empty() || db.get_font(span.font_id).is_none() {
                            continue;
                        }
                        let mut shape_text = part.shape_text;
                        let mut sources = vec![ShapedSourcePart {
                            span_idx: part.span_idx,
                            byte_range: part.byte_range,
                            text_range: 0..shape_text.len(),
                        }];
                        while segment_pieces.peek().is_some_and(|next| {
                            text_parts_can_shape_together(spans, part.span_idx, next)
                        }) {
                            let Some(SegmentPiece::Text(next)) = segment_pieces.next() else {
                                break;
                            };
                            let text_start = shape_text.len();
                            shape_text.push_str(&next.shape_text);
                            sources.push(ShapedSourcePart {
                                span_idx: next.span_idx,
                                byte_range: next.byte_range,
                                text_range: text_start..shape_text.len(),
                            });
                        }
                        let runs = shaper.shape_text(&shape_text, span.font_id, span.font_size);
                        let part_width: f32 = runs.iter().map(|run| run.width).sum();
                        width += part_width;
                        pieces.push(ShapedPiece::Text(ShapedPart {
                            sources,
                            shape_text,
                            runs,
                        }));
                    }
                    SegmentPiece::InlineObject(object) => {
                        width += object.width;
                        pieces.push(ShapedPiece::InlineObject(object));
                    }
                }
            }
            let trailing_space_width =
                trailing_space_width(&pieces, options.trailing_space_behavior);
            let has_non_space_content = pieces.iter().any(|piece| match piece {
                ShapedPiece::Text(part) => part
                    .shape_text
                    .chars()
                    .any(|ch| !is_line_end_space_char(ch, options.trailing_space_behavior)),
                ShapedPiece::InlineObject(_) => true,
            });
            shaped_segments.push(ShapedSegment {
                pieces,
                width,
                trailing_space_width,
                has_non_space_content,
                metric_span_idx: seg.metric_span_idx,
                mandatory_after: seg.mandatory_after,
                is_newline_break: seg.is_newline_break,
            });
        }

        let limit = max_width.filter(|w| w.is_finite() && *w >= 0.0);

        for seg in shaped_segments {
            let metric_span = spans.get(seg.metric_span_idx);

            if seg.is_newline_break {
                let (a, d, h) = metric_span
                    .map(|span| line_metrics_for_span(db, span))
                    .unwrap_or((0.0, 0.0, 0.0));
                if line_items.is_empty() {
                    line_ascent = line_ascent.max(a);
                    line_descent = line_descent.max(d);
                    line_height = line_height.max(h);
                }
                flush_line(
                    &mut lines,
                    &mut line_items,
                    &mut line_objects,
                    &mut trailing_space_indices,
                    &mut inline_objects,
                    &mut line_width,
                    &mut current_y,
                    &mut line_ascent,
                    &mut line_descent,
                    &mut line_height,
                    align,
                    limit,
                    LineEnd::ForcedBreak,
                    options.strut,
                );
                continue;
            }

            if let Some(lim) = limit.filter(|_| options.wrap) {
                let current_trailing_width: f32 = trailing_space_indices
                    .iter()
                    .filter_map(|index| line_items.get(*index))
                    .map(|item| item.advance)
                    .sum();
                let candidate_width =
                    if options.trailing_space_behavior != TrailingSpaceBehavior::Preserve {
                        line_width + seg.width
                            - seg.trailing_space_width
                            - if seg.has_non_space_content {
                                0.0
                            } else {
                                current_trailing_width
                            }
                    } else {
                        line_width + seg.width
                    };
                if (!line_items.is_empty() || !line_objects.is_empty()) && candidate_width > lim {
                    flush_line(
                        &mut lines,
                        &mut line_items,
                        &mut line_objects,
                        &mut trailing_space_indices,
                        &mut inline_objects,
                        &mut line_width,
                        &mut current_y,
                        &mut line_ascent,
                        &mut line_descent,
                        &mut line_height,
                        align,
                        limit,
                        LineEnd::SoftWrap,
                        options.strut,
                    );
                }
            }

            for piece in seg.pieces {
                match piece {
                    ShapedPiece::Text(part) => {
                        let Some(first_source) = part.sources.first() else {
                            continue;
                        };
                        let Some(span) = spans.get(first_source.span_idx) else {
                            continue;
                        };
                        for source in &part.sources {
                            if let Some(source_span) = spans.get(source.span_idx) {
                                let (a, d, h) = line_metrics_for_span(db, source_span);
                                line_ascent = line_ascent.max(a);
                                line_descent = line_descent.max(d);
                                line_height = line_height.max(h);
                            }
                        }
                        let trailing_start =
                            trailing_space_start(&part.shape_text, options.trailing_space_behavior);
                        if part
                            .shape_text
                            .chars()
                            .any(|ch| !is_line_end_space_char(ch, options.trailing_space_behavior))
                        {
                            trailing_space_indices.clear();
                        }

                        let segment_end = u32::try_from(part.shape_text.len()).unwrap_or(u32::MAX);
                        let mut cluster_starts: Vec<_> = part
                            .runs
                            .iter()
                            .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.cluster))
                            .collect();
                        cluster_starts.sort_unstable();
                        cluster_starts.dedup();
                        for run in part.runs {
                            let mut actual_span = span.clone();
                            actual_span.font_id = run.font_id;
                            let (a, d, h) = line_metrics_for_span(db, &actual_span);
                            line_ascent = line_ascent.max(a);
                            line_descent = line_descent.max(d);
                            line_height = line_height.max(h);
                            let mut run_x = line_width;
                            for g in run.glyphs {
                                let next_cluster = cluster_starts
                                    .get(
                                        cluster_starts.partition_point(|start| *start <= g.cluster),
                                    )
                                    .copied()
                                    .unwrap_or(segment_end);
                                let source_ranges = source_ranges_for_cluster(
                                    &part.sources,
                                    g.cluster as usize,
                                    next_cluster.max(g.cluster) as usize,
                                );
                                let source = source_ranges.first().copied();
                                let source_span = source
                                    .and_then(|source| spans.get(source.span_index))
                                    .unwrap_or(span);
                                let additional_sources = (source_ranges.len() > 1)
                                    .then(|| source_ranges[1..].to_vec().into_boxed_slice());
                                line_items.push(LineItem {
                                    glyph_id: g.glyph_id,
                                    x: run_x + g.x_offset,
                                    y_offset: g.y_offset,
                                    cluster: source.map_or(g.cluster, |source| source.byte_start),
                                    source,
                                    additional_sources,
                                    font_id: run.font_id,
                                    font_size: source_span.font_size,
                                    color: source_span.color,
                                    advance: g.x_advance,
                                    is_space: is_space_glyph(&part.shape_text, g.cluster as usize),
                                    is_collapsible_space: is_collapsible_space_glyph(
                                        &part.shape_text,
                                        g.cluster as usize,
                                    ),
                                    trailing_space_behavior: source_span
                                        .trailing_space_behavior
                                        .unwrap_or(options.trailing_space_behavior),
                                });
                                if source_span
                                    .trailing_space_behavior
                                    .unwrap_or(options.trailing_space_behavior)
                                    != TrailingSpaceBehavior::Preserve
                                    && trailing_start.is_some_and(|start| {
                                        (g.cluster as usize) >= start
                                            && is_line_end_space_glyph(
                                                &part.shape_text,
                                                g.cluster as usize,
                                                options.trailing_space_behavior,
                                            )
                                    })
                                {
                                    trailing_space_indices.push(line_items.len() - 1);
                                }
                                run_x += g.x_advance;
                            }
                            line_width += run.width;
                        }
                    }
                    ShapedPiece::InlineObject(object) => {
                        trailing_space_indices.clear();
                        line_objects.push(LineObject {
                            metrics: object,
                            x: line_width,
                        });
                        line_width += object.width;
                        line_ascent = line_ascent.max(object.baseline);
                        line_descent = line_descent.max((object.height - object.baseline).max(0.0));
                        line_height = line_height.max(line_ascent + line_descent);
                    }
                }
            }
            line_height = line_height.max(line_ascent + line_descent);

            if seg.mandatory_after {
                flush_line(
                    &mut lines,
                    &mut line_items,
                    &mut line_objects,
                    &mut trailing_space_indices,
                    &mut inline_objects,
                    &mut line_width,
                    &mut current_y,
                    &mut line_ascent,
                    &mut line_descent,
                    &mut line_height,
                    align,
                    limit,
                    LineEnd::ForcedBreak,
                    options.strut,
                );
            }
        }

        if !line_items.is_empty()
            || !line_objects.is_empty()
            || lines.is_empty() && line_width == 0.0
        {
            if !line_items.is_empty() || !line_objects.is_empty() {
                flush_line(
                    &mut lines,
                    &mut line_items,
                    &mut line_objects,
                    &mut trailing_space_indices,
                    &mut inline_objects,
                    &mut line_width,
                    &mut current_y,
                    &mut line_ascent,
                    &mut line_descent,
                    &mut line_height,
                    align,
                    limit,
                    LineEnd::ParagraphEnd,
                    options.strut,
                );
            }
        }

        let total_width = lines.iter().map(|l| l.width).fold(0.0f32, f32::max);
        let total_height = current_y;

        Self {
            lines,
            inline_objects,
            width: total_width,
            height: total_height,
        }
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<u32> {
        self.closest_glyph(x, y).map(|glyph| glyph.cluster)
    }

    pub fn hit_test_source(&self, x: f32, y: f32) -> Option<TextSourceRange> {
        self.closest_glyph(x, y)?.source
    }

    fn closest_glyph(&self, x: f32, y: f32) -> Option<&LayoutGlyph> {
        if self.lines.is_empty() {
            return None;
        }

        let target_line = self
            .lines
            .iter()
            .find(|line| y >= line.y && y <= line.y + line.height)
            .or_else(|| {
                if y < self.lines[0].y {
                    self.lines.first()
                } else {
                    self.lines.last()
                }
            })?;

        if target_line.glyphs.is_empty() {
            return None;
        }

        let mut closest = &target_line.glyphs[0];
        let mut best = (target_line.glyphs[0].x - x).abs();
        for g in &target_line.glyphs[1..] {
            let dist = (g.x - x).abs();
            if dist < best {
                best = dist;
                closest = g;
            }
        }

        Some(closest)
    }
}

struct LineItem {
    glyph_id: u32,
    x: f32,
    y_offset: f32,
    cluster: u32,
    source: Option<TextSourceRange>,
    additional_sources: Option<Box<[TextSourceRange]>>,
    font_id: FontId,
    font_size: f32,
    color: [f32; 4],
    advance: f32,
    is_space: bool,
    is_collapsible_space: bool,
    trailing_space_behavior: TrailingSpaceBehavior,
}

#[derive(Debug, Clone, Copy)]
struct LineObject {
    metrics: InlineObjectMetrics,
    x: f32,
}

#[allow(clippy::too_many_arguments)]
fn flush_line(
    lines: &mut Vec<LayoutLine>,
    items: &mut Vec<LineItem>,
    objects: &mut Vec<LineObject>,
    trailing_space_indices: &mut Vec<usize>,
    positioned_objects: &mut Vec<PositionedInlineObject>,
    line_width: &mut f32,
    current_y: &mut f32,
    ascent: &mut f32,
    descent: &mut f32,
    height: &mut f32,
    align: TextAlign,
    max_width: Option<f32>,
    line_end: LineEnd,
    strut: Option<LineStrut>,
) {
    if items.is_empty() && objects.is_empty() {
        let h = if *height > 0.0 { *height } else { 0.0 };
        let a = *ascent;
        let d = *descent;
        if h > 0.0 {
            let baseline = *current_y + a;
            lines.push(LayoutLine {
                glyphs: Vec::new(),
                y: *current_y,
                width: 0.0,
                height: h,
                ascent: a,
                descent: d,
                baseline,
            });
            *current_y += h;
        }
        *line_width = 0.0;
        trailing_space_indices.clear();
        *ascent = 0.0;
        *descent = 0.0;
        *height = 0.0;
        return;
    }

    if let Some(strut) = strut {
        *ascent = ascent.max(strut.ascent);
        *descent = descent.max(strut.descent);
        *height = height.max(strut.height).max(*ascent + *descent);
    }

    let raw_width = *line_width;
    let mut hanging_width = 0.0;
    for index in trailing_space_indices.drain(..) {
        if let Some(item) = items.get_mut(index) {
            let hangs = match item.trailing_space_behavior {
                TrailingSpaceBehavior::Remove => true,
                TrailingSpaceBehavior::Hang if line_end == LineEnd::SoftWrap => true,
                TrailingSpaceBehavior::Hang => max_width.is_some_and(|width| raw_width > width),
                TrailingSpaceBehavior::Preserve => false,
            };
            if hangs {
                hanging_width += item.advance;
                if item.trailing_space_behavior == TrailingSpaceBehavior::Remove
                    && item.is_collapsible_space
                {
                    item.advance = 0.0;
                }
                item.is_space = false;
            }
        }
    }
    *line_width = (*line_width - hanging_width).max(0.0);

    let raw_width = *line_width;

    let mut width = raw_width;
    if align == TextAlign::Justify {
        if let Some(lim) = max_width {
            if line_end == LineEnd::SoftWrap && raw_width < lim {
                let spaces = items.iter().filter(|it| it.is_space).count();
                if spaces > 0 {
                    let extra = lim - raw_width;
                    let per_space = extra / spaces as f32;
                    let mut shift = 0.0;
                    let mut space_positions = Vec::with_capacity(spaces);
                    for it in items.iter_mut() {
                        let original_x = it.x;
                        it.x += shift;
                        if it.is_space {
                            space_positions.push(original_x);
                            it.advance += per_space;
                            shift += per_space;
                        }
                    }
                    for object in objects.iter_mut() {
                        let preceding_spaces = space_positions
                            .iter()
                            .filter(|space_x| **space_x < object.x)
                            .count();
                        object.x += preceding_spaces as f32 * per_space;
                    }
                    width = lim;
                }
            }
        }
    }

    let baseline = *current_y + *ascent;

    let x_offset = if let Some(lim) = max_width {
        match align {
            TextAlign::Left | TextAlign::Justify => 0.0,
            TextAlign::Center => ((lim - width) / 2.0).max(0.0),
            TextAlign::Right => (lim - width).max(0.0),
        }
    } else {
        0.0
    };

    let mut glyphs = Vec::with_capacity(items.len());
    for it in items.drain(..) {
        glyphs.push(LayoutGlyph {
            glyph_id: it.glyph_id,
            x: it.x + x_offset,
            advance: it.advance,
            y: baseline - it.y_offset,
            cluster: it.cluster,
            source: it.source,
            paint: true,
            font_id: it.font_id,
            font_size: it.font_size,
            color: it.color,
        });
        if let Some(sources) = it.additional_sources {
            glyphs.extend(sources.into_vec().into_iter().map(|source| LayoutGlyph {
                glyph_id: it.glyph_id,
                x: it.x + x_offset,
                advance: it.advance,
                y: baseline - it.y_offset,
                cluster: source.byte_start,
                source: Some(source),
                paint: false,
                font_id: it.font_id,
                font_size: it.font_size,
                color: it.color,
            }));
        }
    }

    let line_index = lines.len();
    for object in objects.drain(..) {
        positioned_objects.push(PositionedInlineObject {
            id: object.metrics.id,
            line_index,
            x: object.x + x_offset,
            y: baseline - object.metrics.baseline,
            width: object.metrics.width,
            height: object.metrics.height,
        });
    }

    lines.push(LayoutLine {
        glyphs,
        y: *current_y,
        width,
        height: *height,
        ascent: *ascent,
        descent: *descent,
        baseline,
    });
    *current_y += *height;
    *line_width = 0.0;
    *ascent = 0.0;
    *descent = 0.0;
    *height = 0.0;
}

fn line_metrics_for_span(db: &FontDatabase, span: &TextSpan) -> (f32, f32, f32) {
    let metrics = line_metrics_for_font(db, span.font_id, span.font_size, span.line_height);
    (metrics.ascent, metrics.descent, metrics.height)
}

pub fn line_metrics_for_font(
    db: &FontDatabase,
    font_id: FontId,
    font_size: f32,
    line_height: Option<f32>,
) -> LineStrut {
    let Some(font) = db.get_font(font_id) else {
        return LineStrut {
            ascent: 0.0,
            descent: 0.0,
            height: 0.0,
        };
    };
    let mut a = font.metrics.ascender_px(font_size);
    let mut d = font.metrics.descender_px(font_size).abs();
    let mut h = font.metrics.line_height_px(font_size).max(a + d);
    if let Some(sw) = font.as_swash_font() {
        let m = sw.metrics(&[]).scale(font_size);
        if m.ascent > 0.0 && m.descent != 0.0 {
            let sa = m.ascent;
            let sd = m.descent.abs();
            if (sa - a).abs() > a * 0.5 && sa > 0.0 {
                a = sa;
                d = sd;
                h = (sa + sd + m.leading).max(sa + sd);
            }
        }
    }
    if line_height.is_none() {
        let gap = (h - a - d).max(0.0);
        a = a.round();
        d = d.round();
        h = a + d + gap.round();
    }
    h = line_height.unwrap_or(h);
    let leading = (h - a - d) / 2.0;
    LineStrut {
        ascent: a + leading,
        descent: d + leading,
        height: h,
    }
}

fn text_parts_can_shape_together(
    spans: &[TextSpan],
    left_span_idx: usize,
    next: &SegmentPiece,
) -> bool {
    let SegmentPiece::Text(next) = next else {
        return false;
    };
    let (Some(left), Some(right)) = (spans.get(left_span_idx), spans.get(next.span_idx)) else {
        return false;
    };
    left.font_id == right.font_id && left.font_size == right.font_size
}

fn source_ranges_for_cluster(
    sources: &[ShapedSourcePart],
    cluster_start: usize,
    cluster_end: usize,
) -> Vec<TextSourceRange> {
    let mut ranges = Vec::new();
    for source in sources {
        let overlap_start = cluster_start.max(source.text_range.start);
        let overlap_end = cluster_end.min(source.text_range.end);
        let has_overlap = overlap_start < overlap_end;
        let contains_empty_cluster = cluster_start == cluster_end
            && source.text_range.start <= cluster_start
            && cluster_start < source.text_range.end;
        if !has_overlap && !contains_empty_cluster {
            continue;
        }
        ranges.push(TextSourceRange {
            span_index: source.span_idx,
            byte_start: (source.byte_range.start + overlap_start - source.text_range.start) as u32,
            byte_end: (source.byte_range.start + overlap_end - source.text_range.start) as u32,
        });
    }
    ranges
}

fn split_content_uax14(
    spans: &[TextSpan],
    content: &[ParagraphItem],
    db: &FontDatabase,
    space_break_behavior: SpaceBreakBehavior,
) -> Vec<Segment> {
    use unicode_linebreak::{linebreaks, BreakOpportunity};

    const OBJECT_MARKER: char = '\u{FFFC}';
    let mut text = String::new();
    let mut span_ranges = Vec::with_capacity(spans.len());
    let mut object_ranges = Vec::new();
    for item in content {
        match item {
            ParagraphItem::TextSpan(span_idx) => {
                let Some(span) = spans.get(*span_idx) else {
                    continue;
                };
                if db.get_font(span.font_id).is_none() || span.text.is_empty() {
                    continue;
                }
                let start = text.len();
                text.push_str(&span.text);
                span_ranges.push((*span_idx, start..text.len()));
            }
            ParagraphItem::InlineObject(object) => {
                let start = text.len();
                text.push(OBJECT_MARKER);
                let width = finite_nonnegative(object.width);
                let height = finite_nonnegative(object.height);
                let baseline = finite_nonnegative(object.baseline).min(height);
                object_ranges.push((
                    start..text.len(),
                    InlineObjectMetrics {
                        id: object.id,
                        width,
                        height,
                        baseline,
                    },
                ));
            }
        }
    }
    if text.is_empty() {
        return Vec::new();
    }
    let first_valid_span_idx = spans
        .iter()
        .position(|span| db.get_font(span.font_id).is_some())
        .unwrap_or(0);

    let mut breaks: Vec<(usize, BreakOpportunity)> = linebreaks(&text).collect();
    if space_break_behavior == SpaceBreakBehavior::AfterEachPreservedSpace {
        let mut by_offset = std::collections::BTreeMap::new();
        for (offset, opportunity) in breaks {
            by_offset
                .entry(offset)
                .and_modify(|current| {
                    if opportunity == BreakOpportunity::Mandatory {
                        *current = BreakOpportunity::Mandatory;
                    }
                })
                .or_insert(opportunity);
        }
        for (offset, ch) in text.char_indices() {
            if is_breakable_preserved_space(ch) {
                by_offset
                    .entry(offset + ch.len_utf8())
                    .or_insert(BreakOpportunity::Allowed);
            }
        }
        breaks = by_offset.into_iter().collect();
    }
    if breaks.is_empty() {
        breaks.push((text.len(), BreakOpportunity::Allowed));
    }

    let mut segs = Vec::new();
    let mut prev = 0;
    let mut span_cursor = 0usize;
    let mut object_cursor = 0usize;
    for (pos, op) in breaks {
        if pos < prev || pos > text.len() {
            continue;
        }
        if !text.is_char_boundary(pos) {
            continue;
        }
        let raw = &text[prev..pos];
        if raw.is_empty() {
            prev = pos;
            continue;
        }
        let contains_hard = raw.chars().any(is_hard_break_char);
        let mandatory = op == BreakOpportunity::Mandatory && contains_hard;

        let mut shape_end = pos;
        while shape_end > prev {
            let Some(ch) = text[prev..shape_end].chars().next_back() else {
                break;
            };
            if !is_hard_break_char(ch) {
                break;
            }
            shape_end -= ch.len_utf8();
        }

        while span_cursor < span_ranges.len() && span_ranges[span_cursor].1.end <= prev {
            span_cursor += 1;
        }
        while object_cursor < object_ranges.len() && object_ranges[object_cursor].0.end <= prev {
            object_cursor += 1;
        }
        let metric_span_idx = (span_cursor..span_ranges.len())
            .find_map(|idx| {
                let (span_idx, range) = &span_ranges[idx];
                (range.start < pos && range.end > prev).then_some(*span_idx)
            })
            .unwrap_or(first_valid_span_idx);
        let mut pieces_with_order: Vec<(usize, SegmentPiece)> = Vec::new();
        for (span_idx, span_range) in span_ranges.iter().skip(span_cursor) {
            if span_range.start >= shape_end {
                break;
            }
            let part_start = prev.max(span_range.start);
            let part_end = shape_end.min(span_range.end);
            if part_start >= part_end {
                continue;
            }
            let byte_start = part_start - span_range.start;
            let byte_end = part_end - span_range.start;
            let Some(span) = spans.get(*span_idx) else {
                continue;
            };
            let Some(shape_text) = span.text.get(byte_start..byte_end) else {
                continue;
            };
            pieces_with_order.push((
                part_start,
                SegmentPiece::Text(SegmentPart {
                    span_idx: *span_idx,
                    byte_range: byte_start..byte_end,
                    shape_text: shape_text.to_string(),
                }),
            ));
        }
        for (object_range, object) in object_ranges.iter().skip(object_cursor) {
            if object_range.start >= shape_end {
                break;
            }
            if object_range.start >= prev && object_range.end <= shape_end {
                pieces_with_order.push((object_range.start, SegmentPiece::InlineObject(*object)));
            }
        }
        pieces_with_order.sort_by_key(|(offset, _)| *offset);
        let pieces = pieces_with_order
            .into_iter()
            .map(|(_, piece)| piece)
            .collect();

        let newline_only = shape_end == prev && (mandatory || shape_end < pos);
        segs.push(Segment {
            pieces,
            metric_span_idx,
            mandatory_after: mandatory,
            is_newline_break: newline_only,
        });
        prev = pos;
        while span_cursor < span_ranges.len() && span_ranges[span_cursor].1.end <= prev {
            span_cursor += 1;
        }
        while object_cursor < object_ranges.len() && object_ranges[object_cursor].0.end <= prev {
            object_cursor += 1;
        }
    }
    segs
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn is_collapsible_space_char(ch: char) -> bool {
    matches!(ch, ' ' | '\t')
}

fn is_unicode_space_separator(ch: char) -> bool {
    matches!(
        ch,
        '\u{0020}' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

fn is_breakable_preserved_space(ch: char) -> bool {
    matches!(ch, '\t') || is_unicode_space_separator(ch)
}

fn is_line_end_space_char(ch: char, behavior: TrailingSpaceBehavior) -> bool {
    behavior != TrailingSpaceBehavior::Preserve
        && (is_collapsible_space_char(ch) || is_unicode_space_separator(ch))
}

fn trailing_space_start(text: &str, behavior: TrailingSpaceBehavior) -> Option<usize> {
    if behavior == TrailingSpaceBehavior::Preserve {
        return None;
    }
    let mut start = text.len();
    for (byte, ch) in text.char_indices().rev() {
        if !is_line_end_space_char(ch, behavior) {
            break;
        }
        start = byte;
    }
    (start < text.len()).then_some(start)
}

fn trailing_space_width(pieces: &[ShapedPiece], behavior: TrailingSpaceBehavior) -> f32 {
    if behavior == TrailingSpaceBehavior::Preserve {
        return 0.0;
    }
    let mut width = 0.0;
    for piece in pieces.iter().rev() {
        match piece {
            ShapedPiece::InlineObject(_) => break,
            ShapedPiece::Text(part) => {
                let Some(start) = trailing_space_start(&part.shape_text, behavior) else {
                    break;
                };
                width += part
                    .runs
                    .iter()
                    .flat_map(|run| &run.glyphs)
                    .filter(|glyph| {
                        glyph.cluster as usize >= start
                            && is_line_end_space_glyph(
                                &part.shape_text,
                                glyph.cluster as usize,
                                behavior,
                            )
                    })
                    .map(|glyph| glyph.x_advance)
                    .sum::<f32>();
                if start > 0 {
                    break;
                }
            }
        }
    }
    width
}

fn is_hard_break_char(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

fn is_space_glyph(shaped: &str, cluster_byte: usize) -> bool {
    if cluster_byte >= shaped.len() || !shaped.is_char_boundary(cluster_byte) {
        return false;
    }
    match shaped[cluster_byte..].chars().next() {
        Some(' ') | Some('\t') => true,
        _ => false,
    }
}

fn is_collapsible_space_glyph(shaped: &str, cluster_byte: usize) -> bool {
    char_at_cluster(shaped, cluster_byte).is_some_and(is_collapsible_space_char)
}

fn is_line_end_space_glyph(
    shaped: &str,
    cluster_byte: usize,
    behavior: TrailingSpaceBehavior,
) -> bool {
    char_at_cluster(shaped, cluster_byte).is_some_and(|ch| is_line_end_space_char(ch, behavior))
}

fn char_at_cluster(shaped: &str, cluster_byte: usize) -> Option<char> {
    if cluster_byte >= shaped.len() || !shaped.is_char_boundary(cluster_byte) {
        return None;
    }
    shaped[cluster_byte..].chars().next()
}
