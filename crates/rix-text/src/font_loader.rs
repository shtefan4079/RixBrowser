
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontWeight(pub u16);

impl FontWeight {
    pub const THIN: Self = Self(100);
    pub const EXTRA_LIGHT: Self = Self(200);
    pub const LIGHT: Self = Self(300);
    pub const NORMAL: Self = Self(400);
    pub const MEDIUM: Self = Self(500);
    pub const SEMI_BOLD: Self = Self(600);
    pub const BOLD: Self = Self(700);
    pub const EXTRA_BOLD: Self = Self(800);
    pub const BLACK: Self = Self(900);
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::NORMAL
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontDescriptor {
    pub family: String,
    pub weight: FontWeight,
    pub style: FontStyle,
}

impl FontDescriptor {
    pub fn new(family: impl Into<String>) -> Self {
        Self {
            family: family.into(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }

    pub fn with_weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }

    pub fn with_style(mut self, style: FontStyle) -> Self {
        self.style = style;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    pub cap_height: Option<i16>,
    pub x_height: Option<i16>,
}

impl FontMetrics {
    pub fn scale(&self, font_size: f32) -> f32 {
        font_size / (self.units_per_em as f32).max(1.0)
    }

    pub fn ascender_px(&self, font_size: f32) -> f32 {
        self.ascender as f32 * self.scale(font_size)
    }

    pub fn descender_px(&self, font_size: f32) -> f32 {
        self.descender as f32 * self.scale(font_size)
    }

    pub fn line_height_px(&self, font_size: f32) -> f32 {
        (self.ascender - self.descender + self.line_gap) as f32 * self.scale(font_size)
    }

    pub fn x_height_px(&self, font_size: f32) -> Option<f32> {
        self.x_height
            .map(|height| height as f32 * self.scale(font_size))
            .filter(|height| *height > 0.0)
    }
}

#[derive(Clone)]
pub struct FontData {
    pub id: FontId,
    pub descriptor: FontDescriptor,
    pub bytes: Arc<Vec<u8>>,
    pub face_index: u32,
    pub metrics: FontMetrics,
    blob: linebender_resource_handle::Blob<u8>,
    pub variations: Vec<([u8; 4], f32)>,
    pub normalized_coords: Vec<i16>,
}

impl FontData {
    pub fn blob(&self) -> &linebender_resource_handle::Blob<u8> {
        &self.blob
    }

    pub fn as_rustybuzz_face(&self) -> Option<rustybuzz::Face<'_>> {
        let mut face = rustybuzz::Face::from_slice(&self.bytes, self.face_index)?;
        if !self.variations.is_empty() {
            let variations: Vec<rustybuzz::Variation> = self
                .variations
                .iter()
                .map(|(tag, value)| rustybuzz::Variation {
                    tag: rustybuzz::ttf_parser::Tag::from_bytes(tag),
                    value: *value,
                })
                .collect();
            face.set_variations(&variations);
        }
        Some(face)
    }

    pub fn as_swash_font(&self) -> Option<swash::FontRef<'_>> {
        swash::FontRef::from_index(&self.bytes, self.face_index as usize)
    }

    pub fn as_read_font(&self) -> Result<read_fonts::FontRef<'_>, read_fonts::ReadError> {
        read_fonts::FontRef::from_index(&self.bytes, self.face_index)
    }

    pub fn supports_char(&self, ch: char) -> bool {
        if let Some(font) = self.as_swash_font() {
            let gid = font.charmap().map(ch);
            if gid != 0 {
                return true;
            }
            if ch == '\0' {
                return true;
            }
        }
        if let Ok(rf) = self.as_read_font() {
            use read_fonts::TableProvider;
            if let Ok(cmap) = rf.cmap() {
                if cmap.map_codepoint(ch as u32).is_some() {
                    return true;
                }
            }
        }
        false
    }
}

#[derive(Clone)]
pub struct FontDatabase {
    pub(crate) fonts: Vec<FontData>,
    pub(crate) family_index: HashMap<String, Vec<FontId>>,
    pub(crate) aliases: Arc<crate::fontconfig::FamilyAliases>,
    pub default_sans_serif: Option<FontId>,
    pub default_serif: Option<FontId>,
    pub default_monospace: Option<FontId>,
    pub default_emoji: Option<FontId>,
}

impl Default for FontDatabase {
    fn default() -> Self {
        Self {
            fonts: Vec::new(),
            family_index: HashMap::new(),
            aliases: Arc::default(),
            default_sans_serif: None,
            default_serif: None,
            default_monospace: None,
            default_emoji: None,
        }
    }
}

static SYSTEM_FONTS_CACHE: std::sync::OnceLock<FontDatabase> = std::sync::OnceLock::new();

impl FontDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    pub fn get_font(&self, id: FontId) -> Option<&FontData> {
        self.fonts.get(id.0)
    }

    pub fn find_covering_font(&self, ch: char, preferred: Option<FontId>) -> Option<FontId> {
        if let Some(id) = preferred {
            if let Some(f) = self.get_font(id) {
                if f.supports_char(ch) {
                    return Some(id);
                }
            }
        }
        if is_emoji_char(ch) {
            if let Some(id) = self.default_emoji {
                if let Some(f) = self.get_font(id) {
                    if f.supports_char(ch) {
                        return Some(id);
                    }
                }
            }
        }
        for f in &self.fonts {
            if f.supports_char(ch) {
                return Some(f.id);
            }
        }
        None
    }

    pub fn add_font_bytes(&mut self, bytes: Vec<u8>, face_index: u32) -> Result<FontId, String> {
        use read_fonts::TableProvider;
        let read_font = read_fonts::FontRef::from_index(&bytes, face_index)
            .map_err(|e| format!("Failed to parse font data (read-fonts): {e:?}"))?;
        let _ = read_font
            .head()
            .map_err(|e| format!("Missing head table: {e:?}"))?;

        let swash_font = swash::FontRef::from_index(&bytes, face_index as usize)
            .ok_or_else(|| "Failed to parse font data (swash)".to_string())?;

        let family =
            extract_family_name(&swash_font, &read_font).unwrap_or_else(|| "Unknown".to_string());

        let attrs = swash_font.attributes();
        let style = match attrs.style() {
            swash::Style::Normal => FontStyle::Normal,
            swash::Style::Italic => FontStyle::Italic,
            swash::Style::Oblique(_) => FontStyle::Oblique,
        };
        let raw_weight = attrs.weight().0;
        let weight = FontWeight(raw_weight.clamp(100, 900));

        let metrics = extract_metrics(&swash_font, &read_font);

        let id = FontId(self.fonts.len());
        let descriptor = FontDescriptor {
            family: family.clone(),
            weight,
            style,
        };

        let weight_instances = variable_weight_instances(&swash_font, weight);
        let bytes = Arc::new(bytes);
        let blob = linebender_resource_handle::Blob::new(bytes.clone());
        let data = FontData {
            id,
            descriptor: descriptor.clone(),
            bytes: bytes.clone(),
            face_index,
            metrics,
            blob: blob.clone(),
            variations: Vec::new(),
            normalized_coords: Vec::new(),
        };

        self.fonts.push(data);
        self.family_index
            .entry(family.to_lowercase())
            .or_default()
            .push(id);

        for (instance_weight, normalized_coords) in weight_instances {
            let instance_id = FontId(self.fonts.len());
            self.fonts.push(FontData {
                id: instance_id,
                descriptor: FontDescriptor {
                    weight: FontWeight(instance_weight),
                    ..descriptor.clone()
                },
                bytes: bytes.clone(),
                face_index,
                metrics,
                blob: blob.clone(),
                variations: vec![(*b"wght", instance_weight as f32)],
                normalized_coords,
            });
            self.family_index
                .entry(family.to_lowercase())
                .or_default()
                .push(instance_id);
        }

        self.auto_assign_fallback(id, &family);

        Ok(id)
    }

    pub fn load_font_file<P: AsRef<Path>>(&mut self, path: P) -> Result<Vec<FontId>, String> {
        let bytes = std::fs::read(path.as_ref()).map_err(|e| {
            format!(
                "Failed to read font file '{}': {e}",
                path.as_ref().display()
            )
        })?;

        let num_faces = count_faces(&bytes);
        let mut ids = Vec::new();

        for face_idx in 0..num_faces {
            if let Ok(id) = self.add_font_bytes(bytes.clone(), face_idx) {
                ids.push(id);
            }
        }

        if ids.is_empty() {
            Err(format!(
                "No valid font faces found in '{}'",
                path.as_ref().display()
            ))
        } else {
            Ok(ids)
        }
    }

    pub fn scan_system_fonts(&mut self) -> usize {
        if self.fonts.is_empty() {
            let cached = SYSTEM_FONTS_CACHE.get_or_init(|| {
                let mut db = FontDatabase::new();
                db.scan_system_fonts_with_limit(32);
                db
            });
            *self = cached.clone();
            return self.fonts.len();
        }
        self.scan_system_fonts_with_limit(32)
    }

    pub fn scan_system_fonts_with_limit(&mut self, limit: usize) -> usize {
        let mut paths_to_scan = vec![
            PathBuf::from("/usr/share/fonts/Adwaita"),
            PathBuf::from("/usr/share/fonts/cantarell"),
            PathBuf::from("/usr/share/fonts/liberation"),
            PathBuf::from("/usr/share/fonts/noto"),
            PathBuf::from("/usr/share/fonts/TTF"),
            PathBuf::from("/usr/share/fonts/truetype"),
            PathBuf::from("/usr/share/fonts"),
            PathBuf::from("/usr/local/share/fonts"),
        ];

        if let Ok(home) = std::env::var("HOME") {
            let home_path = PathBuf::from(home);
            paths_to_scan.push(home_path.join(".local/share/fonts"));
            paths_to_scan.push(home_path.join(".fonts"));
        }

        let mut count = 0;
        for root in paths_to_scan {
            if root.exists() {
                self.scan_dir_recursive(&root, &mut count, limit);
                if count >= limit {
                    break;
                }
            }
        }

        if let Some(id) = self.best_fallback_matching(|font| {
            let family = font.descriptor.family.to_lowercase();
            (family.contains("sans") || family.contains("adwaita") || family.contains("dejavu"))
                && !family.contains("mono")
                && !family.contains("serif")
        }) {
            self.default_sans_serif = Some(id);
        }
        if let Some(id) = self.best_fallback_matching(|font| {
            let family = font.descriptor.family.to_lowercase();
            family.contains("serif") && !family.contains("sans")
        }) {
            self.default_serif = Some(id);
        }
        if let Some(id) = self.best_fallback_matching(|font| {
            let family = font.descriptor.family.to_lowercase();
            family.contains("mono") || family.contains("console") || family.contains("typewriter")
        }) {
            self.default_monospace = Some(id);
        }
        if let Some(id) = self.best_fallback_matching(|font| {
            let family = font.descriptor.family.to_lowercase();
            family.contains("emoji") || family.contains("color")
        }) {
            self.default_emoji = Some(id);
        }

        self.set_family_aliases(crate::fontconfig::FamilyAliases::from_system());

        if count > 0 {
            let first = self.fonts.first().map(|f| f.id);
            if self.default_sans_serif.is_none() {
                self.default_sans_serif = first;
            }
            if self.default_serif.is_none() {
                self.default_serif = first;
            }
            if self.default_monospace.is_none() {
                self.default_monospace = first;
            }
        }

        tracing::info!("Discovered and loaded {count} system fonts");
        count
    }

    pub fn set_family_aliases(&mut self, aliases: crate::fontconfig::FamilyAliases) {
        self.aliases = Arc::new(aliases);
        let normal = FontDescriptor::new("");
        if let Some(id) = self.match_family("sans-serif", &normal) {
            self.default_sans_serif = Some(id);
        }
        if let Some(id) = self.match_family("serif", &normal) {
            self.default_serif = Some(id);
        }
        if let Some(id) = self.match_family("monospace", &normal) {
            self.default_monospace = Some(id);
        }
        if let Some(id) = self.match_family("emoji", &normal) {
            self.default_emoji = Some(id);
        }
    }

    fn best_fallback_matching(
        &self,
        mut matches_family: impl FnMut(&FontData) -> bool,
    ) -> Option<FontId> {
        self.fonts
            .iter()
            .filter(|font| matches_family(font))
            .min_by_key(|font| {
                let style_penalty = match font.descriptor.style {
                    FontStyle::Normal => 0,
                    FontStyle::Oblique => 1,
                    FontStyle::Italic => 2,
                };
                let weight_distance =
                    (font.descriptor.weight.0 as i32 - FontWeight::NORMAL.0 as i32).unsigned_abs();
                (style_penalty, weight_distance, font.id)
            })
            .map(|font| font.id)
    }

    fn scan_dir_recursive(&mut self, dir: &Path, count: &mut usize, limit: usize) {
        if *count >= limit {
            return;
        }

        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return,
        };

        for entry in read_dir.flatten() {
            if *count >= limit {
                break;
            }
            let path = entry.path();
            if path.to_string_lossy().contains("cjk") {
                continue;
            }

            if path.is_dir() {
                self.scan_dir_recursive(&path, count, limit);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                if ext_lower == "ttf" || ext_lower == "otf" || ext_lower == "ttc" {
                    if let Ok(ids) = self.load_font_file(&path) {
                        *count += ids.len();
                    }
                }
            }
        }
    }

    fn auto_assign_fallback(&mut self, id: FontId, family: &str) {
        let lower = family.to_lowercase();

        if self.default_emoji.is_none() && (lower.contains("emoji") || lower.contains("color")) {
            self.default_emoji = Some(id);
        } else if self.default_monospace.is_none()
            && (lower.contains("mono") || lower.contains("console") || lower.contains("typewriter"))
        {
            self.default_monospace = Some(id);
        } else if self.default_serif.is_none()
            && (lower.contains("serif") && !lower.contains("sans"))
        {
            self.default_serif = Some(id);
        } else if self.default_sans_serif.is_none()
            && (lower.contains("sans") || lower.contains("adwaita") || lower.contains("dejavu"))
        {
            self.default_sans_serif = Some(id);
        }
    }
}

fn count_faces(bytes: &[u8]) -> u32 {
    match read_fonts::FileRef::new(bytes) {
        Ok(read_fonts::FileRef::Font(_)) => 1,
        Ok(read_fonts::FileRef::Collection(col)) => col.len().max(1),
        Err(_) => 1,
    }
}

fn extract_family_name(
    swash_font: &swash::FontRef<'_>,
    read_font: &read_fonts::FontRef<'_>,
) -> Option<String> {
    use read_fonts::TableProvider;
    use swash::StringId;

    let mut full_fallback: Option<String> = None;
    let mut family_fallback: Option<String> = None;

    for s in swash_font.localized_strings() {
        if !s.is_decodable() {
            continue;
        }
        let text: String = s.chars().collect();
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        match s.id() {
            StringId::TypographicFamily | StringId::WwsFamily => {
                return Some(text);
            }
            StringId::Family => {
                if family_fallback.is_none() {
                    family_fallback = Some(text);
                }
            }
            StringId::Full => {
                if full_fallback.is_none() {
                    full_fallback = Some(text);
                }
            }
            _ => {}
        }
    }
    if let Some(f) = family_fallback {
        return Some(f);
    }

    if let Ok(name) = read_font.name() {
        use read_fonts::types::NameId;
        let data = name.string_data();
        let mut id1: Option<String> = None;
        let mut id4: Option<String> = None;
        for rec in name.name_record() {
            let nid = rec.name_id();
            if nid == NameId::TYPOGRAPHIC_FAMILY_NAME
                || nid == NameId::FAMILY_NAME
                || nid == NameId::FULL_NAME
            {
                if let Ok(ns) = rec.string(data) {
                    let text: String = ns.chars().collect();
                    let text = text.trim().to_string();
                    if text.is_empty() {
                        continue;
                    }
                    let unicode = rec.is_unicode();
                    if nid == NameId::TYPOGRAPHIC_FAMILY_NAME {
                        if unicode {
                            return Some(text);
                        }
                        if id1.is_none() {
                            id4 = Some(text);
                        }
                    } else if nid == NameId::FAMILY_NAME {
                        if unicode {
                            if id1.is_none() {
                                id1 = Some(text);
                            }
                        } else if id1.is_none() {
                            id1 = Some(text);
                        }
                    } else if id4.is_none() {
                        id4 = Some(text);
                    }
                }
            }
        }
        if let Some(f) = id1 {
            return Some(f);
        }
        if let Some(f) = id4 {
            return Some(f);
        }
    }

    full_fallback
}

fn variable_weight_instances(
    font: &swash::FontRef<'_>,
    default_weight: FontWeight,
) -> Vec<(u16, Vec<i16>)> {
    let variations = font.variations();
    let wght = swash::tag_from_bytes(b"wght");
    let Some(axis) = variations.find_by_tag(wght) else {
        return Vec::new();
    };
    let (min, max) = (axis.min_value(), axis.max_value());
    if !(min < max) {
        return Vec::new();
    }
    (1..=9u16)
        .map(|step| step * 100)
        .filter(|weight| *weight != default_weight.0)
        .filter(|weight| (min..=max).contains(&(*weight as f32)))
        .map(|weight| {
            let coords = variations
                .normalized_coords([(wght, weight as f32)])
                .collect();
            (weight, coords)
        })
        .collect()
}

fn extract_metrics(
    swash_font: &swash::FontRef<'_>,
    read_font: &read_fonts::FontRef<'_>,
) -> FontMetrics {
    use read_fonts::TableProvider;

    let m = swash_font.metrics(&[]);
    let mut upem = m.units_per_em;
    let mut asc = m.ascent as i16;
    let mut desc = if m.descent > 0.0 {
        -(m.descent as i16)
    } else {
        m.descent as i16
    };
    if m.ascent < 0.0 {
        asc = m.ascent.abs() as i16;
    }
    let gap = m.leading as i16;
    let mut cap = if m.cap_height != 0.0 {
        Some(m.cap_height as i16)
    } else {
        None
    };
    let mut xh = if m.x_height != 0.0 {
        Some(m.x_height as i16)
    } else {
        None
    };

    if upem == 0 {
        if let Ok(head) = read_font.head() {
            upem = head.units_per_em();
        }
    }
    if upem == 0 {
        upem = 1000;
    }
    if asc == 0 {
        if let Ok(hhea) = read_font.hhea() {
            asc = hhea.ascender().to_i16();
        }
    }
    if desc == 0 {
        if let Ok(hhea) = read_font.hhea() {
            desc = hhea.descender().to_i16();
        }
    }
    if cap.is_none() || xh.is_none() {
        if let Ok(os2) = read_font.os2() {
            if cap.is_none() {
                if let Some(v) = os2.s_cap_height() {
                    if v != 0 {
                        cap = Some(v);
                    }
                }
            }
            if xh.is_none() {
                if let Some(v) = os2.sx_height() {
                    if v != 0 {
                        xh = Some(v);
                    }
                }
            }
        }
    }
    if asc < 0 {
        asc = -asc;
    }
    if desc > 0 {
        desc = -desc;
    }
    if asc == 0 {
        asc = (upem as i16 * 4) / 5;
    }
    if desc == 0 {
        desc = -(upem as i16 / 5);
    }

    FontMetrics {
        units_per_em: upem,
        ascender: asc,
        descender: desc,
        line_gap: gap,
        cap_height: cap,
        x_height: xh,
    }
}

pub(crate) fn is_emoji_char(ch: char) -> bool {
    let v = ch as u32;
    matches!(
        v,
        0x1F300..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0xFE00..=0xFE0F | 0x1F000..=0x1F02F | 0x1F100..=0x1F1FF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_fonts_cache_reuse() {
        let mut db1 = FontDatabase::new();
        let count1 = db1.scan_system_fonts();

        let mut db2 = FontDatabase::new();
        let count2 = db2.scan_system_fonts();

        assert_eq!(count1, count2);
        assert_eq!(db1.len(), db2.len());
        assert_eq!(db1.default_sans_serif, db2.default_sans_serif);
    }
}
