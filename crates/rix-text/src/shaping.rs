
use crate::font_loader::{FontDatabase, FontId};
use std::sync::Arc;
use unicode_bidi::BidiInfo;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone)]
struct ShapeCacheEntry {
    font_bytes_ptr: usize,
    face_index: u32,
    font_size_bits: u32,
    text: String,
    runs: Vec<ShapedRun>,
}

fn compute_shape_hash(font_bytes_ptr: usize, face_index: u32, font_size_bits: u32, text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    font_bytes_ptr.hash(&mut hasher);
    face_index.hash(&mut hasher);
    font_size_bits.hash(&mut hasher);
    text.hash(&mut hasher);
    hasher.finish()
}

thread_local! {
    static SHAPED_RUN_CACHE: std::cell::RefCell<std::collections::HashMap<u64, Vec<ShapeCacheEntry>>> =
        std::cell::RefCell::new(std::collections::HashMap::with_capacity(4096));
}

pub fn contains_rtl_runs(text: &str) -> bool {
    BidiInfo::new(text, None)
        .levels
        .iter()
        .any(|level| level.is_rtl())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphPosition {
    pub glyph_id: u32,
    pub cluster: u32,
    pub x_advance: f32,
    pub y_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShapedRun {
    pub font_id: FontId,
    pub text_range: std::ops::Range<usize>,
    pub direction: Direction,
    pub glyphs: Vec<GlyphPosition>,
    pub width: f32,
    pub font_size: f32,
}

pub struct TextShaper<'a> {
    db: &'a FontDatabase,
}

impl<'a> TextShaper<'a> {
    pub fn new(db: &'a FontDatabase) -> Self {
        Self { db }
    }

    pub fn shape_text(&self, text: &str, font_id: FontId, font_size: f32) -> Vec<ShapedRun> {
        if text.is_empty() {
            return Vec::new();
        }
        let Some(font) = self.db.get_font(font_id) else {
            return Vec::new();
        };

        let can_cache = text.len() <= 512;
        let (font_bytes_ptr, face_index, font_size_bits) = (
            Arc::as_ptr(&font.bytes) as usize,
            font.face_index,
            font_size.to_bits(),
        );
        let hash = if can_cache {
            let h = compute_shape_hash(font_bytes_ptr, face_index, font_size_bits, text);
            if let Ok(guard) = SHAPED_RUN_CACHE.try_with(|c| {
                c.borrow().get(&h).and_then(|entries| {
                    entries.iter().find(|e| {
                        e.font_bytes_ptr == font_bytes_ptr
                            && e.face_index == face_index
                            && e.font_size_bits == font_size_bits
                            && e.text == text
                    }).map(|e| e.runs.clone())
                })
            }) {
                if let Some(cached) = guard {
                    return cached;
                }
            }
            Some(h)
        } else {
            None
        };

        let features = typographic_features();
        let bidi_info = BidiInfo::new(text, None);
        let mut visual_runs = Vec::new();

        if bidi_info.paragraphs.is_empty() {
            if let Some(run) = self.shape_single_run(
                text,
                0..text.len(),
                Direction::Ltr,
                font_id,
                font_size,
                &features,
            ) {
                visual_runs.push(run);
            }
        } else {
            for para in &bidi_info.paragraphs {
                let (_, runs) = bidi_info.visual_runs(para, para.range.clone());

                for run_range in runs {
                    let is_rtl = bidi_info.levels[run_range.start].is_rtl();
                    let direction = if is_rtl {
                        Direction::Rtl
                    } else {
                        Direction::Ltr
                    };
                    let mut font_runs = self.split_by_font_coverage(text, run_range.clone(), font_id);
                    if is_rtl {
                        font_runs.reverse();
                    }
                    for (sub_range, sub_font) in font_runs {
                        if let Some(shaped) = self.shape_single_run(
                            text, sub_range, direction, sub_font, font_size, &features,
                        ) {
                            visual_runs.push(shaped);
                        }
                    }
                }
            }
        }

        if let Some(h) = hash {
            let _ = SHAPED_RUN_CACHE.try_with(|c| {
                let mut guard = c.borrow_mut();
                if guard.len() >= 16384 {
                    guard.clear();
                }
                let entries = guard.entry(h).or_default();
                if entries.len() < 4 {
                    entries.push(ShapeCacheEntry {
                        font_bytes_ptr,
                        face_index,
                        font_size_bits,
                        text: text.to_string(),
                        runs: visual_runs.clone(),
                    });
                }
            });
        }

        visual_runs
    }

    fn shape_single_run(
        &self,
        text: &str,
        byte_range: std::ops::Range<usize>,
        direction: Direction,
        font_id: FontId,
        font_size: f32,
        features: &[rustybuzz::Feature],
    ) -> Option<ShapedRun> {
        let font = self.db.get_font(font_id)?;
        let rb_face = font.as_rustybuzz_face()?;
        let scale = font.metrics.scale(font_size);
        let run_text = text.get(byte_range.clone())?;
        if run_text.is_empty() {
            return None;
        }
        let is_rtl = direction == Direction::Rtl;

        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(run_text);
        buffer.set_direction(if is_rtl {
            rustybuzz::Direction::RightToLeft
        } else {
            rustybuzz::Direction::LeftToRight
        });

        let glyph_buffer = rustybuzz::shape(&rb_face, features, buffer);
        let infos = glyph_buffer.glyph_infos();
        let positions = glyph_buffer.glyph_positions();

        let mut glyphs = Vec::with_capacity(infos.len());
        let mut run_width = 0.0;

        for (info, pos) in infos.iter().zip(positions.iter()) {
            let x_adv = pos.x_advance as f32 * scale;
            let y_adv = pos.y_advance as f32 * scale;
            let x_off = pos.x_offset as f32 * scale;
            let y_off = pos.y_offset as f32 * scale;

            run_width += x_adv;
            glyphs.push(GlyphPosition {
                glyph_id: info.glyph_id,
                cluster: info.cluster + byte_range.start as u32,
                x_advance: x_adv,
                y_advance: y_adv,
                x_offset: x_off,
                y_offset: y_off,
            });
        }

        Some(ShapedRun {
            font_id,
            text_range: byte_range,
            direction,
            glyphs,
            width: run_width,
            font_size,
        })
    }

    fn split_by_font_coverage(
        &self,
        text: &str,
        range: std::ops::Range<usize>,
        primary: FontId,
    ) -> Vec<(std::ops::Range<usize>, FontId)> {
        let Some(slice) = text.get(range.clone()) else {
            return Vec::new();
        };
        let base = range.start;
        let mut out: Vec<(std::ops::Range<usize>, FontId)> = Vec::new();

        let mut seg_start = base;
        let mut seg_font = primary;
        let mut seg_end = base;
        let mut prev_font = primary;

        for (rel_offset, grapheme) in slice.grapheme_indices(true) {
            let abs_start = base + rel_offset;
            let abs_end = abs_start + grapheme.len();
            let font_for_cluster = self.font_for_grapheme(grapheme, primary, prev_font);

            if out.is_empty() && seg_end == seg_start {
                seg_font = font_for_cluster;
                seg_start = abs_start;
                seg_end = abs_end;
            } else if font_for_cluster == seg_font {
                seg_end = abs_end;
            } else {
                out.push((seg_start..seg_end, seg_font));
                seg_start = abs_start;
                seg_end = abs_end;
                seg_font = font_for_cluster;
            }
            prev_font = font_for_cluster;
        }
        if seg_end > seg_start {
            out.push((seg_start..seg_end, seg_font));
        }
        out
    }

    fn font_for_grapheme(&self, grapheme: &str, primary: FontId, prev: FontId) -> FontId {
        let mut has_base = false;
        let mut all_primary = true;
        for ch in grapheme.chars() {
            if is_inheriting_mark(ch) {
                continue;
            }
            has_base = true;
            if !covers(self.db, primary, ch) {
                all_primary = false;
                break;
            }
        }
        if !has_base {
            return prev;
        }
        if all_primary {
            return primary;
        }
        for ch in grapheme.chars() {
            if is_inheriting_mark(ch) {
                continue;
            }
            if !covers(self.db, primary, ch) {
                if let Some(id) = self.db.find_covering_font(ch, Some(primary)) {
                    return id;
                }
                break;
            }
        }
        primary
    }
}

fn typographic_features() -> Vec<rustybuzz::Feature> {
    use std::str::FromStr;
    const ENABLED: &[&str] = &[
        "+liga", "+clig", "+calt", "+kern", "+mark", "+mkmk", "+dist", "+abvm", "+blwm", "+curs",
        "+locl",
    ];
    ENABLED
        .iter()
        .filter_map(|s| rustybuzz::Feature::from_str(s).ok())
        .collect()
}

fn covers(db: &FontDatabase, id: FontId, ch: char) -> bool {
    db.get_font(id)
        .map(|f| f.supports_char(ch))
        .unwrap_or(false)
}

fn is_inheriting_mark(ch: char) -> bool {
    let v = ch as u32;
    matches!(
        v,
        0x0300..=0x036F
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F
        | 0x200C..=0x200D
        | 0x1F3FB..=0x1F3FF
        | 0xE0020..=0xE007F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shape_text_caching_dedup() {
        let mut db = FontDatabase::new();
        db.scan_system_fonts();
        let Some(font_id) = db.default_sans_serif else {
            return;
        };
        let shaper = TextShaper::new(&db);
        let runs1 = shaper.shape_text("Hello YouTube world!", font_id, 16.0);
        assert!(!runs1.is_empty(), "First shaping run must return glyphs");
        let runs2 = shaper.shape_text("Hello YouTube world!", font_id, 16.0);
        assert_eq!(runs1, runs2, "Second shaping run must match cached run exactly");
    }
}
