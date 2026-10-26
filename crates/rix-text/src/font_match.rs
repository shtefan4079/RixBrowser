
use crate::font_loader::{FontDatabase, FontDescriptor, FontId, FontStyle};

const MAX_ALIAS_DEPTH: usize = 4;

pub(crate) const GENERIC_FAMILIES: [&str; 9] = [
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "emoji",
    "math",
    "fangsong",
];

fn is_generic(family: &str) -> bool {
    GENERIC_FAMILIES.iter().any(|generic| family.eq_ignore_ascii_case(generic))
}

fn generic_defaults(generic: &str) -> &'static [&'static str] {
    match generic {
        "serif" => &["Times New Roman", "serif"],
        "sans-serif" => &["Arial", "sans-serif"],
        "monospace" => &["monospace"],
        "cursive" => &["Comic Sans MS", "cursive", "sans-serif"],
        "fantasy" => &["Impact", "fantasy", "sans-serif"],
        "system-ui" => &["sans-serif"],
        "emoji" => &["emoji"],
        "math" => &["math", "serif"],
        "fangsong" => &["fangsong", "serif"],
        _ => &[],
    }
}

pub fn parse_family_list(value: &str) -> Vec<String> {
    let mut families = Vec::new();
    let mut family = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in value.chars() {
        if escaped {
            family.push(character);
            escaped = false;
        } else if character == '\\' {
            family.push(character);
            escaped = true;
        } else if let Some(quote_character) = quote {
            family.push(character);
            if character == quote_character {
                quote = None;
            }
        } else if character == '"' || character == '\'' {
            family.push(character);
            quote = Some(character);
        } else if character == ',' {
            push_normalized_family(&mut families, &family);
            family.clear();
        } else {
            family.push(character);
        }
    }
    push_normalized_family(&mut families, &family);
    families
}

fn push_normalized_family(families: &mut Vec<String>, raw_family: &str) {
    let raw_family = raw_family.trim();
    let family = if raw_family.len() >= 2
        && ((raw_family.starts_with('"') && raw_family.ends_with('"'))
            || (raw_family.starts_with('\'') && raw_family.ends_with('\'')))
    {
        decode_css_escapes(&raw_family[1..raw_family.len() - 1])
    } else {
        decode_css_escapes(raw_family)
    };
    let normalized = family.split_whitespace().collect::<Vec<_>>().join(" ");
    if !normalized.is_empty() {
        families.push(normalized);
    }
}

fn decode_css_escapes(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();

    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }

        let Some(next) = characters.next() else {
            continue;
        };
        if next.is_ascii_hexdigit() {
            let mut code_point = String::with_capacity(6);
            code_point.push(next);
            while code_point.len() < 6
                && characters
                    .peek()
                    .is_some_and(|character| character.is_ascii_hexdigit())
            {
                if let Some(character) = characters.next() {
                    code_point.push(character);
                }
            }
            if characters
                .peek()
                .is_some_and(|character| character.is_whitespace())
            {
                let consumed_whitespace = characters.next();
                if consumed_whitespace == Some('\r') && characters.peek() == Some(&'\n') {
                    characters.next();
                }
            }
            let scalar = u32::from_str_radix(&code_point, 16).unwrap_or(0xfffd);
            output.push(char::from_u32(scalar).unwrap_or('\u{fffd}'));
        } else if next == '\n' || next == '\r' || next == '\u{000c}' {
            if next == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
            }
        } else {
            output.push(next);
        }
    }

    output
}

impl FontDatabase {
    pub fn match_font(&self, descriptor: &FontDescriptor) -> Option<FontId> {
        for family in parse_family_list(&descriptor.family) {
            if let Some(id) = self.match_family(&family, descriptor) {
                return Some(id);
            }
        }
        self.default_sans_serif
            .or(self.default_serif)
            .and_then(|id| self.same_family_best(id, descriptor))
            .or_else(|| self.fonts.first().map(|font| font.id))
    }

    pub fn match_family(&self, family: &str, descriptor: &FontDescriptor) -> Option<FontId> {
        let lower = family.to_lowercase();
        if is_generic(&lower) {
            for name in generic_defaults(&lower) {
                let found = if is_generic(name) {
                    self.resolve_aliases(name, descriptor, 0)
                } else {
                    self.resolve_named(name, descriptor, 0)
                };
                if found.is_some() {
                    return found;
                }
            }
            let fallback = match lower.as_str() {
                "serif" | "math" | "fangsong" => self.default_serif,
                "monospace" => self.default_monospace,
                "emoji" => self.default_emoji,
                _ => self.default_sans_serif,
            };
            return fallback.and_then(|id| self.same_family_best(id, descriptor));
        }
        self.resolve_named(family, descriptor, 0)
            .or_else(|| self.substitute_named(family, descriptor))
    }

    fn substitute_named(&self, family: &str, descriptor: &FontDescriptor) -> Option<FontId> {
        let lower = family.to_lowercase();
        let mono = ["mono", "code", "console", "typewriter"]
            .iter()
            .any(|hint| lower.contains(hint));
        let serif = [
            "serif", "roman", "times", "georgia", "garamond", "palatino", "cambria",
            "constantia", "bookman", "didot", "baskerville",
        ]
        .iter()
        .any(|hint| lower.contains(hint));
        let sans = [
            "sans", "gothic", "helvetica", "arial", "verdana", "tahoma", "roboto", "inter",
            "segoe", "ubuntu", "system", "avenir", "futura", "montserrat", "poppins",
        ]
        .iter()
        .any(|hint| lower.contains(hint));
        let fallback = if mono {
            self.default_monospace
        } else if serif {
            self.default_serif
        } else if sans {
            self.default_sans_serif
        } else {
            None
        };
        fallback.and_then(|id| self.same_family_best(id, descriptor))
    }

    fn resolve_named(&self, family: &str, descriptor: &FontDescriptor, depth: usize) -> Option<FontId> {
        if let Some(candidates) = self.family_index.get(&family.to_lowercase()) {
            return self.best_match_in_candidates(candidates, descriptor);
        }
        self.resolve_aliases(family, descriptor, depth)
    }

    fn resolve_aliases(&self, family: &str, descriptor: &FontDescriptor, depth: usize) -> Option<FontId> {
        if depth >= MAX_ALIAS_DEPTH {
            return None;
        }
        self.aliases
            .families(family)
            .iter()
            .filter(|alias| !is_generic(alias) && !alias.eq_ignore_ascii_case(family))
            .find_map(|alias| self.resolve_named(alias, descriptor, depth + 1))
    }

    fn same_family_best(&self, id: FontId, descriptor: &FontDescriptor) -> Option<FontId> {
        let font = self.get_font(id)?;
        match self.family_index.get(&font.descriptor.family.to_lowercase()) {
            Some(candidates) => self.best_match_in_candidates(candidates, descriptor),
            None => Some(id),
        }
    }

    pub(crate) fn best_match_in_candidates(
        &self,
        candidates: &[FontId],
        descriptor: &FontDescriptor,
    ) -> Option<FontId> {
        let faces: Vec<_> = candidates.iter().filter_map(|id| self.get_font(*id)).collect();
        let style_order: [FontStyle; 3] = match descriptor.style {
            FontStyle::Italic => [FontStyle::Italic, FontStyle::Oblique, FontStyle::Normal],
            FontStyle::Oblique => [FontStyle::Oblique, FontStyle::Italic, FontStyle::Normal],
            FontStyle::Normal => [FontStyle::Normal, FontStyle::Oblique, FontStyle::Italic],
        };
        let style = style_order
            .into_iter()
            .find(|style| faces.iter().any(|face| face.descriptor.style == *style))?;
        let faces: Vec<_> = faces
            .into_iter()
            .filter(|face| face.descriptor.style == style)
            .collect();
        let desired = descriptor.weight.0 as i32;
        faces
            .iter()
            .min_by_key(|face| (weight_rank(desired, face.descriptor.weight.0 as i32), face.id.0))
            .map(|face| face.id)
    }
}

fn weight_rank(desired: i32, available: i32) -> (u8, i32) {
    if available == desired {
        return (0, 0);
    }
    if (400..=500).contains(&desired) {
        if available > desired && available <= 500 {
            return (1, available - desired);
        }
        if available < desired {
            return (2, desired - available);
        }
        return (3, available - desired);
    }
    if desired > 500 {
        if available > desired {
            (1, available - desired)
        } else {
            (2, desired - available)
        }
    } else if available < desired {
        (1, desired - available)
    } else {
        (2, available - desired)
    }
}
