
#[derive(Debug, Clone, PartialEq)]
pub struct ImageCandidate<'a> {
    pub url: &'a str,
    pub density: Option<f32>,
    pub width: Option<u32>,
}

pub fn parse_srcset(srcset: &str) -> Vec<ImageCandidate<'_>> {
    let mut candidates = Vec::new();
    let mut remaining = srcset.trim();

    while !remaining.is_empty() {
        remaining = remaining.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if remaining.is_empty() {
            break;
        }

        let url_end = if remaining.starts_with("data:") {
            remaining
                .find(|c: char| c.is_ascii_whitespace())
                .unwrap_or(remaining.len())
        } else {
            remaining
                .find(|c: char| c.is_ascii_whitespace() || c == ',')
                .unwrap_or(remaining.len())
        };

        let url = remaining[..url_end].trim();
        remaining = remaining[url_end..].trim_start();

        let mut density = None;
        let mut width = None;

        if !remaining.is_empty() && !remaining.starts_with(',') {
            let desc_end = remaining.find(',').unwrap_or(remaining.len());
            let descriptor = remaining[..desc_end].trim();
            remaining = &remaining[desc_end..];

            for token in descriptor.split_whitespace() {
                if let Some(d) = token.strip_suffix('x').or_else(|| token.strip_suffix('X')) {
                    if let Ok(val) = d.parse::<f32>() {
                        if val > 0.0 && val.is_finite() {
                            density = Some(val);
                        }
                    }
                } else if let Some(w) = token.strip_suffix('w').or_else(|| token.strip_suffix('W'))
                {
                    if let Ok(val) = w.parse::<u32>() {
                        if val > 0 {
                            width = Some(val);
                        }
                    }
                }
            }
        }

        if density.is_some() && width.is_some() {
            continue;
        }
        if density.is_none() && width.is_none() {
            density = Some(1.0);
        }

        if !url.is_empty() {
            candidates.push(ImageCandidate {
                url,
                density,
                width,
            });
        }
    }

    candidates
}

pub fn select_image_source<'a>(src: Option<&'a str>, srcset: Option<&'a str>) -> Option<&'a str> {
    if let Some(srcset) = srcset {
        let candidates = parse_srcset(srcset);
        if !candidates.is_empty() {
            let candidate_1x = candidates
                .iter()
                .find(|c| c.density.map_or(false, |d| (d - 1.0).abs() < 0.1));
            let candidate_w = candidates
                .iter()
                .find(|c| c.width.map_or(false, |w| (300..=1280).contains(&w)));

            let chosen = candidate_1x.or(candidate_w).or_else(|| candidates.first());

            if let Some(chosen) = chosen {
                return Some(chosen.url);
            }
        }
    }

    src.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}
