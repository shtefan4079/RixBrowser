
use rix_dom::query::ComplexSelector;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

static MEDIA_QUERY_CACHE: OnceLock<RwLock<HashMap<(u64, u32, u32), bool>>> = OnceLock::new();
static SUPPORTS_QUERY_CACHE: OnceLock<RwLock<HashMap<u64, bool>>> = OnceLock::new();
static INLINE_STYLE_CACHE: OnceLock<RwLock<HashMap<String, Arc<Vec<Declaration>>>>> = OnceLock::new();

#[inline]
fn css_query_hash(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub name: rix_core::InternedStr,
    pub value: String,
    pub important: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub selectors: Vec<ComplexSelector>,
    pub declarations: Vec<Declaration>,
    pub media: Option<String>,
    pub layer: Option<String>,
    pub container: Option<String>,
    pub container_name: Option<String>,
    pub supports: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleSheet {
    pub rules: Vec<Rule>,
    pub layer_order: Vec<String>,
}

impl StyleSheet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn layers(&self) -> &[String] {
        &self.layer_order
    }

    pub fn parse(css: &str) -> Arc<Self> {
        Self::parse_with_media(css, None)
    }

    pub fn parse_with_media(css: &str, current_media: Option<&str>) -> Arc<Self> {
        let key = stylesheet_cache_key(css, current_media);

        {
            let guard = stylesheet_cache()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(cached) = guard.get(&key) {
                return cached.clone();
            }
        }

        let mut sheet = parse_via_lightningcss(css)
            .unwrap_or_else(|| parse_legacy(css, None, None, None, None));

        if let Some(outer) = current_media {
            for rule in &mut sheet.rules {
                rule.media = Some(match rule.media.take() {
                    Some(inner) => format!("{outer} and ({inner})"),
                    None => outer.to_string(),
                });
            }
        }
        let arc = Arc::new(sheet);

        let mut guard = stylesheet_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = match guard.get(&key) {
            Some(existing) => existing.clone(),
            None => {
                guard.insert(key, arc.clone());
                arc
            }
        };
        result
    }
}

static STYLESHEET_CACHE: OnceLock<Mutex<HashMap<u64, Arc<StyleSheet>>>> = OnceLock::new();

fn stylesheet_cache() -> &'static Mutex<HashMap<u64, Arc<StyleSheet>>> {
    STYLESHEET_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn stylesheet_cache_key(css: &str, current_media: Option<&str>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    css.hash(&mut hasher);
    if let Some(media) = current_media {
        media.hash(&mut hasher);
    }
    hasher.finish()
}

struct FlattenContext {
    media: Option<String>,
    layer: Option<String>,
    container: Option<String>,
    container_name: Option<String>,
    supports: Option<String>,
    parent_selector: Option<String>,
    layer_order: Vec<String>,
    anonymous_layers: usize,
    rules: Vec<Rule>,
}

impl FlattenContext {
    fn new() -> Self {
        Self {
            media: None,
            layer: None,
            container: None,
            container_name: None,
            supports: None,
            parent_selector: None,
            layer_order: Vec::new(),
            anonymous_layers: 0,
            rules: Vec::new(),
        }
    }

    fn declare_layer(&mut self, name: Option<String>) -> Option<String> {
        let effective = match name {
            Some(n) => n,
            None => {
                let gen = format!("(anonymous-{})", self.anonymous_layers);
                self.anonymous_layers += 1;
                gen
            }
        };
        if !self.layer_order.iter().any(|l| l == &effective) {
            self.layer_order.push(effective.clone());
        }
        Some(effective)
    }
}

fn parse_via_lightningcss(css: &str) -> Option<StyleSheet> {
    use lightningcss::stylesheet::{ParserOptions, StyleSheet as LightningSheet};

    if css.trim().is_empty() {
        return Some(StyleSheet::default());
    }

    let options = ParserOptions {
        filename: "style.css".into(),
        error_recovery: true,
        ..Default::default()
    };
    let sheet = LightningSheet::parse(css, options).ok()?;

    let mut ctx = FlattenContext::new();
    flatten_lightning_list(&sheet.rules.0, &mut ctx);

    if ctx.rules.is_empty() && !css.trim().is_empty() {
        let legacy = parse_legacy(css, None, None, None, None);
        if !legacy.rules.is_empty() {
            return Some(legacy);
        }
    }

    Some(StyleSheet {
        rules: ctx.rules,
        layer_order: ctx.layer_order,
    })
}

fn flatten_lightning_list(rules: &[lightningcss::rules::CssRule], ctx: &mut FlattenContext) {
    use lightningcss::rules::CssRule;

    for rule in rules {
        match rule {
            CssRule::Style(style) => lower_lightning_style(style, ctx),
            CssRule::Media(media) => {
                flatten_lightning_media(&media.query, &media.rules, ctx);
            }
            CssRule::Supports(supports) => {
                let cond = css_serialize(&supports.condition);
                let combined = combine_supports(ctx.supports.as_deref(), &cond);
                let saved = ctx.supports.take();
                ctx.supports = Some(combined);
                flatten_lightning_list(&supports.rules.0, ctx);
                ctx.supports = saved;
            }
            CssRule::Container(container) => {
                let cond = container
                    .condition
                    .as_ref()
                    .map(css_serialize)
                    .filter(|s| !s.trim().is_empty() && s.trim() != "none");
                let name = container.name.as_ref().map(css_serialize);
                let saved_container = ctx.container.clone();
                let saved_name = ctx.container_name.clone();
                ctx.container =
                    combine_condition(saved_container.as_deref(), cond.as_deref().unwrap_or(""));
                if name.is_some() {
                    ctx.container_name = name;
                }
                flatten_lightning_list(&container.rules.0, ctx);
                ctx.container = saved_container;
                ctx.container_name = saved_name;
            }
            CssRule::LayerBlock(block) => {
                let name = block.name.as_ref().map(css_serialize);
                let effective = ctx.declare_layer(name);
                let saved = ctx.layer.take();
                ctx.layer = effective;
                flatten_lightning_list(&block.rules.0, ctx);
                ctx.layer = saved;
            }
            CssRule::LayerStatement(stmt) => {
                for name in &stmt.names {
                    ctx.declare_layer(Some(css_serialize(name)));
                }
            }
            CssRule::Nesting(nesting) => {
                lower_lightning_style(&nesting.style, ctx);
            }
            CssRule::Scope(scope) => {
                let prefix = scope.scope_start.as_ref().map(css_serialize);
                let saved_parent = ctx.parent_selector.take();
                ctx.parent_selector = match (saved_parent.clone(), prefix) {
                    (Some(outer), Some(inner)) => Some(format!("{outer} {inner}")),
                    (Some(outer), None) => Some(outer),
                    (None, inner) => inner,
                };
                flatten_lightning_list(&scope.rules.0, ctx);
                ctx.parent_selector = saved_parent;
            }
            CssRule::StartingStyle(starting) => {
                flatten_lightning_list(&starting.rules.0, ctx);
            }
            CssRule::Ignored | CssRule::Unknown(_) | CssRule::Custom(_) => {}
            _ => {}
        }
    }
}

fn flatten_lightning_media(
    query: &lightningcss::media_query::MediaList,
    rules: &lightningcss::rules::CssRuleList,
    ctx: &mut FlattenContext,
) {
    let cond = css_serialize(query);
    let saved = ctx.media.take();
    ctx.media = combine_condition(saved.as_deref(), &cond);
    flatten_lightning_list(&rules.0, ctx);
    ctx.media = saved;
}

fn lower_lightning_style(style: &lightningcss::rules::style::StyleRule, ctx: &mut FlattenContext) {
    let mut selector_text = css_serialize(&style.selectors);
    if selector_text.trim().is_empty() {
        return;
    }
    if let Some(parent) = ctx.parent_selector.as_deref() {
        selector_text = combine_nested_selector(parent, &selector_text);
    }

    let selectors = match ComplexSelector::parse(&selector_text) {
        Ok(list) if !list.is_empty() => list,
        _ => {
            if !style.rules.0.is_empty() {
                let saved = ctx.parent_selector.take();
                ctx.parent_selector = Some(selector_text);
                flatten_lightning_list(&style.rules.0, ctx);
                ctx.parent_selector = saved;
            }
            return;
        }
    };

    let declarations = declarations_from_block(&style.declarations);
    if !declarations.is_empty() {
        ctx.rules.push(Rule {
            selectors,
            declarations,
            media: ctx.media.clone(),
            layer: ctx.layer.clone(),
            container: ctx.container.clone(),
            container_name: ctx.container_name.clone(),
            supports: ctx.supports.clone(),
        });
    }

    if !style.rules.0.is_empty() {
        let inner = css_serialize(&style.selectors);
        let saved = ctx.parent_selector.clone();
        let combined = match saved.as_deref() {
            Some(outer) => combine_nested_selector(outer, &inner),
            None => inner,
        };
        ctx.parent_selector = Some(combined);
        flatten_lightning_list(&style.rules.0, ctx);
        ctx.parent_selector = saved;
    }
}

fn css_serialize<T: lightningcss::traits::ToCss>(value: &T) -> String {
    use lightningcss::printer::{Printer, PrinterOptions};
    let mut out = String::new();
    let mut printer = Printer::new(&mut out, PrinterOptions::default());
    match value.to_css(&mut printer) {
        Ok(()) => out.trim().to_string(),
        Err(_) => String::new(),
    }
}

fn combine_condition(outer: Option<&str>, inner: &str) -> Option<String> {
    let inner = inner.trim();
    match (
        outer.map(str::trim).filter(|s| !s.is_empty()),
        inner.is_empty(),
    ) {
        (None, true) => None,
        (None, false) => Some(inner.to_string()),
        (Some(_), true) => outer.map(str::to_string),
        (Some(o), false) => Some(format!("{o} and ({inner})")),
    }
}

fn combine_supports(outer: Option<&str>, inner: &str) -> String {
    let inner = inner.trim();
    match outer.map(str::trim).filter(|s| !s.is_empty()) {
        None => inner.to_string(),
        Some(o) if inner.is_empty() => o.to_string(),
        Some(o) => format!("({o}) and ({inner})"),
    }
}

fn combine_nested_selector(parent: &str, child_list: &str) -> String {
    let mut out = Vec::new();
    for branch in split_top_level_commas(child_list) {
        let b = branch.trim();
        if b.is_empty() {
            continue;
        }
        if b.contains('&') {
            out.push(b.replace('&', parent));
        } else {
            out.push(format!("{parent} {b}"));
        }
    }
    out.join(", ")
}

fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(s[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(s[start..].to_string());
    parts
}

fn declarations_from_block(
    block: &lightningcss::declaration::DeclarationBlock,
) -> Vec<Declaration> {
    let mut out = Vec::with_capacity(block.declarations.len() + block.important_declarations.len());
    for prop in &block.declarations {
        push_lightning_prop(prop, false, &mut out);
    }
    for prop in &block.important_declarations {
        push_lightning_prop(prop, true, &mut out);
    }
    out
}

fn push_lightning_prop(
    prop: &lightningcss::properties::Property,
    important: bool,
    out: &mut Vec<Declaration>,
) {
    use lightningcss::printer::PrinterOptions;
    let Ok(serialized) = prop.to_css_string(false, PrinterOptions::default()) else {
        return;
    };
    let Some((name_part, value_part)) = serialized.split_once(':') else {
        return;
    };
    let raw_name = name_part.trim();
    let name = if raw_name.starts_with("--") {
        rix_core::InternedStr::from(raw_name)
    } else {
        rix_core::InternedStr::from(raw_name.to_lowercase())
    };
    if name.is_empty() {
        return;
    }
    out.push(Declaration {
        name,
        value: value_part.trim().to_string(),
        important,
    });
}

fn parse_legacy(
    css: &str,
    current_media: Option<&str>,
    current_layer: Option<&str>,
    current_container: Option<&str>,
    current_supports: Option<&str>,
) -> StyleSheet {
    let clean = strip_css_comments(css);
    let mut sheet = StyleSheet::default();
    let mut rules = Vec::new();
    let bytes = clean.as_bytes();
    let mut cursor = 0;

    while cursor < bytes.len() {
        let open_brace = match clean[cursor..].find('{') {
            Some(idx) => cursor + idx,
            None => break,
        };

        let selector_str = clean[cursor..open_brace].trim();

        let mut depth = 1;
        let mut close_brace = None;
        let mut i = open_brace + 1;
        while i < bytes.len() {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        close_brace = Some(i);
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }

        let close_brace = match close_brace {
            Some(idx) => idx,
            None => break,
        };

        let inner_block = clean[open_brace + 1..close_brace].to_string();
        cursor = close_brace + 1;

        let lowered = selector_str.trim_start().to_lowercase();
        if lowered.starts_with("@media") {
            let media_cond = selector_str["@media".len()..].trim();
            let combined = match current_media {
                Some(parent) => format!("{parent} and ({media_cond})"),
                None => media_cond.to_string(),
            };
            let nested = parse_legacy(
                &inner_block,
                Some(&combined),
                current_layer,
                current_container,
                current_supports,
            );
            sheet.layer_order = merge_layer_order(sheet.layer_order, nested.layer_order);
            rules.extend(nested.rules);
        } else if lowered.starts_with("@supports") {
            let cond = selector_str["@supports".len()..].trim().to_string();
            let combined = match current_supports {
                Some(parent) => format!("({parent}) and ({cond})"),
                None => cond,
            };
            let nested = parse_legacy(
                &inner_block,
                current_media,
                current_layer,
                current_container,
                Some(&combined),
            );
            sheet.layer_order = merge_layer_order(sheet.layer_order, nested.layer_order);
            rules.extend(nested.rules);
        } else if lowered.starts_with("@container") {
            let cond = selector_str["@container".len()..].trim().to_string();
            let (name, condition) = split_container_prelude(&cond);
            let combined_container = match current_container {
                Some(parent) if !condition.is_empty() => {
                    format!("{parent} and ({condition})")
                }
                Some(parent) => parent.to_string(),
                None => condition,
            };
            let nested = parse_legacy(
                &inner_block,
                current_media,
                current_layer,
                Some(&combined_container),
                current_supports,
            );
            for rule in nested.rules {
                rules.push(Rule {
                    container_name: rule.container_name.or_else(|| name.clone()),
                    ..rule
                });
            }
            sheet.layer_order = merge_layer_order(sheet.layer_order, nested.layer_order);
        } else if lowered.starts_with("@layer") {
            let rest = selector_str["@layer".len()..].trim();
            if rest.is_empty() || inner_block.trim().is_empty() && rest.ends_with(';') {
                continue;
            }
            let layer_name = rest.trim().to_string();
            if !sheet.layer_order.iter().any(|l| l == &layer_name) && !layer_name.is_empty() {
                sheet.layer_order.push(layer_name.clone());
            }
            let nested = parse_legacy(
                &inner_block,
                current_media,
                Some(&layer_name),
                current_container,
                current_supports,
            );
            sheet.layer_order = merge_layer_order(sheet.layer_order, nested.layer_order);
            rules.extend(nested.rules);
        } else if selector_str.trim_start().starts_with('@') {
            continue;
        } else {
            let mut sel_text = selector_str.to_string();
            if let Some(semi) = sel_text.rfind(';') {
                let (stmt, rest) = sel_text.split_at(semi + 1);
                if stmt.trim_start().to_lowercase().starts_with("@layer") {
                    for name in stmt["@layer".len()..]
                        .trim()
                        .trim_end_matches(';')
                        .split(',')
                    {
                        let n = name.trim().to_string();
                        if !n.is_empty() && !sheet.layer_order.iter().any(|l| l == &n) {
                            sheet.layer_order.push(n);
                        }
                    }
                    sel_text = rest.to_string();
                    if sel_text.trim().is_empty() {
                        continue;
                    }
                }
            }
            match ComplexSelector::parse(sel_text.trim()) {
                Ok(selectors) if !selectors.is_empty() => {
                    let declarations = parse_declarations(&inner_block);
                    if !declarations.is_empty() {
                        rules.push(Rule {
                            selectors,
                            declarations,
                            media: current_media.map(str::to_string),
                            layer: current_layer.map(str::to_string),
                            container: current_container.map(str::to_string),
                            container_name: None,
                            supports: current_supports.map(str::to_string),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    sheet.rules = rules;
    sheet
}

fn merge_layer_order(mut a: Vec<String>, b: Vec<String>) -> Vec<String> {
    for name in b {
        if !a.iter().any(|l| l == &name) {
            a.push(name);
        }
    }
    a
}

fn split_container_prelude(prelude: &str) -> (Option<String>, String) {
    let t = prelude.trim();
    if t.is_empty() {
        return (None, String::new());
    }
    if let Some(paren) = t.find('(') {
        let head = t[..paren].trim();
        if head.is_empty() {
            (None, t.to_string())
        } else {
            (Some(head.to_string()), t[paren..].trim().to_string())
        }
    } else {
        (None, t.to_string())
    }
}

pub fn matches_media_query(query: &str, viewport_width: f32, viewport_height: f32) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let key = (
        css_query_hash(q),
        viewport_width.to_bits(),
        viewport_height.to_bits(),
    );
    let cache = MEDIA_QUERY_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(guard) = cache.read() {
        if let Some(&matched) = guard.get(&key) {
            return matched;
        }
    }
    let matched = matches_media_query_uncached(q, viewport_width, viewport_height);
    if let Ok(mut guard) = cache.write() {
        if guard.len() < 2048 {
            guard.insert(key, matched);
        }
    }
    matched
}

fn matches_media_query_uncached(q: &str, viewport_width: f32, viewport_height: f32) -> bool {
    for single in split_media_list(q) {
        if matches_single_media_query(single.trim(), viewport_width, viewport_height) {
            return true;
        }
    }
    false
}

fn split_media_list(q: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in q.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(q[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(q[start..].to_string());
    parts
}

fn matches_single_media_query(query: &str, vw: f32, vh: f32) -> bool {
    let mut q = query.trim().to_lowercase();
    if q.is_empty() || q == "all" {
        return true;
    }

    let mut negated = false;
    if let Some(rest) = q.strip_prefix("not ") {
        negated = true;
        q = rest.trim().to_string();
    } else if let Some(rest) = q.strip_prefix("not(") {
        negated = true;
        q = format!("({rest}");
    }
    if let Some(rest) = q.strip_prefix("only ") {
        q = rest.trim().to_string();
    }

    let mut type_matches = true;
    for media_type in ["print", "speech", "tty", "tv", "projection", "handheld"] {
        if q == media_type
            || q.starts_with(&format!("{media_type} and "))
            || q.starts_with(&format!("{media_type}("))
        {
            type_matches = false;
            break;
        }
    }
    if q == "screen" {
        q = String::new();
    } else if let Some(rest) = q.strip_prefix("screen and ") {
        q = rest.to_string();
    } else if let Some(rest) = q.strip_prefix("screen(") {
        q = format!("({rest}");
    } else if let Some(rest) = q.strip_prefix("all and ") {
        q = rest.to_string();
    }
    if !type_matches {
        return negated;
    }

    let result = eval_media_features(&q, vw, vh);
    if negated {
        !result
    } else {
        result
    }
}

fn eval_media_features(q: &str, vw: f32, vh: f32) -> bool {
    for clause in split_media_and(q) {
        if !eval_media_clause(clause.trim(), vw, vh) {
            return false;
        }
    }
    true
}

fn split_media_and(q: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let bytes = q.as_bytes();
    let mut start = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && q[i..].starts_with(" and ") {
            parts.push(q[start..i].to_string());
            i += 5;
            start = i;
            continue;
        }
        if q[i..].starts_with(")and(") {
            parts.push(format!("{}))", &q[start..i]));
            i += 4;
            start = i;
            continue;
        }
        i += 1;
    }
    parts.push(q[start..].to_string());
    parts
}

fn eval_media_clause(clause: &str, vw: f32, vh: f32) -> bool {
    let c = clause.trim();
    if c.is_empty() {
        return true;
    }
    if let Some(or_parts) = split_media_or(c) {
        for part in or_parts {
            if eval_media_atom(part.trim(), vw, vh) {
                return true;
            }
        }
        return false;
    }
    eval_media_atom(c, vw, vh)
}

fn split_media_or(c: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let bytes = c.as_bytes();
    let mut start = 0usize;
    let mut found = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth <= 1 && c[i..].starts_with(" or ") {
            parts.push(c[start..i].to_string());
            i += 4;
            start = i;
            found = true;
            continue;
        }
        i += 1;
    }
    if !found {
        return None;
    }
    parts.push(c[start..].to_string());
    Some(parts)
}

fn eval_media_atom(atom: &str, vw: f32, vh: f32) -> bool {
    let mut a = atom.trim();
    if a.starts_with('(') && a.ends_with(')') {
        a = a[1..a.len() - 1].trim();
    }
    if let Some(rest) = a.strip_prefix("not ") {
        return !eval_media_atom(rest.trim(), vw, vh);
    }

    match a {
        "color" | "monochrome" => return true,
        "hover" | "pointer" | "any-hover" | "any-pointer" => return true,
        _ => {}
    }

    if let Some(r) = eval_media_range(a, vw, vh) {
        return r;
    }

    let Some((prop, val)) = a.split_once(':') else {
        return true;
    };
    eval_media_prop(prop.trim(), val.trim(), vw, vh)
}

fn eval_media_prop(prop: &str, val: &str, vw: f32, vh: f32) -> bool {
    let num = parse_css_length_number(val);
    match prop {
        "max-width" => num.map(|n| vw <= n).unwrap_or(true),
        "min-width" => num.map(|n| vw >= n).unwrap_or(true),
        "width" => num.map(|n| (vw - n).abs() < 0.5).unwrap_or(true),
        "max-height" => num.map(|n| vh <= n).unwrap_or(true),
        "min-height" => num.map(|n| vh >= n).unwrap_or(true),
        "height" => num.map(|n| (vh - n).abs() < 0.5).unwrap_or(true),
        "orientation" => match val.trim() {
            "landscape" => vw >= vh,
            "portrait" => vh >= vw,
            _ => true,
        },
        "aspect-ratio" | "min-aspect-ratio" | "max-aspect-ratio" => {
            eval_aspect_ratio(prop, val, vw, vh)
        }
        "resolution" | "min-resolution" | "max-resolution" => eval_resolution(prop, val),
        "color" | "min-color" | "max-color" | "monochrome" | "min-monochrome"
        | "max-monochrome" => eval_color_depth(prop, val),
        "prefers-color-scheme" => val.trim() == "light",
        "forced-colors" => val.trim() == "none",
        "inverted-colors" => val.trim() == "none",
        "prefers-reduced-motion" => val.trim() == "no-preference",
        "prefers-reduced-transparency" | "prefers-reduced-data" => val.trim() == "no-preference",
        "prefers-contrast" => val.trim() == "no-preference",
        "hover" => val.trim() == "hover",
        "any-hover" => val.trim() == "hover",
        "pointer" | "any-pointer" => matches!(val.trim(), "fine" | "coarse"),
        "display-mode" => val.trim() == "browser",
        "scripting" => matches!(val.trim(), "enabled" | "initial-only"),
        "update" => matches!(val.trim(), "fast" | "slow"),
        "overflow-block" | "overflow-inline" => val.trim() == "scroll",
        _ => true,
    }
}

fn eval_media_range(atom: &str, vw: f32, vh: f32) -> Option<bool> {
    let ops = [">=", "<=", ">", "<", "="];
    let tokens: Vec<&str> = atom.split_whitespace().collect();
    if tokens.len() == 5 {
        let (lo_s, op1, feat, op2, hi_s) = (tokens[0], tokens[1], tokens[2], tokens[3], tokens[4]);
        if ops.contains(&op1) && ops.contains(&op2) {
            let dim = media_dimension(feat, vw, vh)?;
            let lo = parse_css_length_number(lo_s)?;
            let hi = parse_css_length_number(hi_s)?;
            let left = compare_range(lo, op1, dim);
            let right = compare_range(dim, op2, hi);
            let _ = left;
            return Some(compare_range(lo, op1, dim) && right);
        }
    }
    if tokens.len() == 3 {
        let (a, op, b) = (tokens[0], tokens[1], tokens[2]);
        if ops.contains(&op) {
            if let Some(dim) = media_dimension(a, vw, vh) {
                let v = parse_css_length_number(b)?;
                return Some(compare_range(dim, op, v));
            }
            if let Some(dim) = media_dimension(b, vw, vh) {
                let v = parse_css_length_number(a)?;
                let flipped = match op {
                    ">=" => "<=",
                    "<=" => ">=",
                    ">" => "<",
                    "<" => ">",
                    _ => "=",
                };
                return Some(compare_range(v, flipped, dim));
            }
        }
    }
    None
}

fn media_dimension(feat: &str, vw: f32, vh: f32) -> Option<f32> {
    match feat {
        "width" => Some(vw),
        "height" => Some(vh),
        _ => None,
    }
}

fn compare_range(lhs: f32, op: &str, rhs: f32) -> bool {
    match op {
        ">=" => lhs >= rhs,
        "<=" => lhs <= rhs,
        ">" => lhs > rhs,
        "<" => lhs < rhs,
        "=" => (lhs - rhs).abs() < 0.5,
        _ => true,
    }
}

fn eval_aspect_ratio(prop: &str, val: &str, vw: f32, vh: f32) -> bool {
    let ratio = parse_ratio(val).unwrap_or(1.0);
    let actual = if vh.abs() < f32::EPSILON {
        f32::MAX
    } else {
        vw / vh
    };
    match prop {
        "min-aspect-ratio" => actual >= ratio,
        "max-aspect-ratio" => actual <= ratio,
        _ => (actual - ratio).abs() < 0.01,
    }
}

fn parse_ratio(val: &str) -> Option<f32> {
    let v = val.trim();
    if let Some((a, b)) = v.split_once('/') {
        let n: f32 = a.trim().parse().ok()?;
        let d: f32 = b.trim().parse().ok()?;
        if d.abs() < f32::EPSILON {
            return None;
        }
        Some(n / d)
    } else {
        v.parse().ok()
    }
}

fn eval_resolution(prop: &str, val: &str) -> bool {
    let dpi = parse_resolution_dpi(val).unwrap_or(96.0);
    match prop {
        "min-resolution" => 96.0 >= dpi,
        "max-resolution" => 96.0 <= dpi,
        _ => (96.0 - dpi).abs() < 1.0,
    }
}

fn parse_resolution_dpi(val: &str) -> Option<f32> {
    let v = val.trim().to_lowercase();
    if let Some(n) = v.strip_suffix("dpi") {
        n.trim().parse().ok()
    } else if let Some(n) = v.strip_suffix("dpcm") {
        let cm: f32 = n.trim().parse().ok()?;
        Some(cm * 2.54)
    } else if let Some(n) = v.strip_suffix("dppx") {
        let x: f32 = n.trim().parse().ok()?;
        Some(x * 96.0)
    } else {
        v.parse().ok()
    }
}

fn eval_color_depth(prop: &str, val: &str) -> bool {
    let bits: f32 = val.trim().parse().unwrap_or(8.0);
    match prop {
        "min-color" | "min-monochrome" => 8.0 >= bits,
        "max-color" | "max-monochrome" => 8.0 <= bits,
        _ => (8.0 - bits).abs() < 0.5,
    }
}

fn parse_css_length_number(val: &str) -> Option<f32> {
    let v = val.trim().to_lowercase();
    for (unit, scale) in [("px", 1.0), ("rem", 16.0), ("em", 16.0)] {
        if let Some(num) = v.strip_suffix(unit) {
            return num.trim().parse::<f32>().ok().map(|n| n * scale);
        }
    }
    v.parse().ok()
}

pub fn matches_container_query(query: &str, container_width: f32, container_height: f32) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let features = strip_container_name(q);
    eval_container_expr(features.trim(), container_width, container_height)
}

fn strip_container_name(q: &str) -> &str {
    let t = q.trim();
    if t.starts_with('(') {
        return t;
    }
    if let Some(paren) = t.find('(') {
        let head = t[..paren].trim();
        if !head.is_empty()
            && !head.eq_ignore_ascii_case("not")
            && !head.contains(' ')
            && head
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return t[paren..].trim();
        }
    }
    t
}

fn eval_container_expr(expr: &str, w: f32, h: f32) -> bool {
    let e = expr.trim();
    if let Some(or_parts) = split_logic_op(e, "or") {
        return or_parts.iter().any(|p| eval_container_expr(p, w, h));
    }
    if let Some(and_parts) = split_logic_op(e, "and") {
        return and_parts.iter().all(|p| eval_container_expr(p, w, h));
    }
    eval_container_atom(e, w, h)
}

fn split_logic_op(expr: &str, op: &str) -> Option<Vec<String>> {
    let needle = format!(" {op} ");
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut found = false;
    let mut i = 0;
    let bytes = expr.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && expr[i..].to_lowercase().starts_with(&needle) {
            parts.push(expr[start..i].to_string());
            i += needle.len();
            start = i;
            found = true;
            continue;
        }
        i += 1;
    }
    if !found {
        return None;
    }
    parts.push(expr[start..].to_string());
    Some(parts)
}

fn eval_container_atom(atom: &str, w: f32, h: f32) -> bool {
    let mut a = atom.trim();
    if a.is_empty() {
        return true;
    }
    if let Some(rest) = a.strip_prefix("not ") {
        return !eval_container_expr(rest.trim(), w, h);
    }
    if a.starts_with('(') && a.ends_with(')') {
        a = a[1..a.len() - 1].trim();
    }
    if a.to_lowercase().starts_with("style(") {
        let inner = a["style(".len()..].trim_end_matches(')').trim();
        if inner.starts_with("--") {
            return true;
        }
        return matches_supports_query(inner);
    }
    if let Some(r) = eval_media_range(a, w, h) {
        return r;
    }
    if let Some((prop, val)) = a.split_once(':') {
        return eval_media_prop(prop.trim().to_lowercase().as_str(), val.trim(), w, h);
    }
    match a.to_lowercase().as_str() {
        "width" | "height" => true,
        _ => true,
    }
}

pub fn matches_supports_query(query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    let key = css_query_hash(q);
    let cache = SUPPORTS_QUERY_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(guard) = cache.read() {
        if let Some(&matched) = guard.get(&key) {
            return matched;
        }
    }
    let matched = eval_supports_expr(q);
    if let Ok(mut guard) = cache.write() {
        if guard.len() < 1024 {
            guard.insert(key, matched);
        }
    }
    matched
}

pub fn supports_declaration(property: &str, value: &str) -> bool {
    let name = property.trim().to_ascii_lowercase();
    let value = value.trim();
    if name.is_empty() || value.is_empty() {
        return false;
    }
    if !name.starts_with("--") && !is_supported_property(&name) {
        return false;
    }

    use lightningcss::properties::{Property, PropertyId};
    use lightningcss::stylesheet::ParserOptions;

    let parsed = Property::parse_string(
        PropertyId::from(name.as_str()),
        value,
        ParserOptions::default(),
    );
    match parsed {
        Ok(Property::Unparsed(_)) => {
            let lowered = value.to_ascii_lowercase();
            lowered.contains("var(") || lowered.contains("env(")
        }
        Ok(_) => true,
        Err(_) => false,
    }
}

fn eval_supports_expr(expr: &str) -> bool {
    let e = expr.trim();
    if e.is_empty() {
        return false;
    }
    if let Some(or_parts) = split_logic_op(e, "or") {
        return or_parts.iter().any(|p| eval_supports_expr(p));
    }
    if let Some(and_parts) = split_logic_op(e, "and") {
        return and_parts.iter().all(|p| eval_supports_expr(p));
    }
    eval_supports_atom(e)
}

fn eval_supports_atom(atom: &str) -> bool {
    let mut a = atom.trim();
    if let Some(rest) = a.strip_prefix("not ") {
        return !eval_supports_expr(rest.trim());
    }
    if a.starts_with('(') && a.ends_with(')') {
        let inner = a[1..a.len() - 1].trim();
        if inner.contains(':') && !inner.starts_with("selector(") {
            return eval_supports_declaration(inner);
        }
        a = inner;
    }
    let low = a.to_lowercase();
    if let Some(inner) = low
        .strip_prefix("selector(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return ComplexSelector::parse(inner.trim()).is_ok();
    }
    if low.starts_with("font-tech(") || low.starts_with("font-format(") {
        return false;
    }
    false
}

fn eval_supports_declaration(decl: &str) -> bool {
    let Some((prop, val)) = decl.split_once(':') else {
        return false;
    };
    supports_declaration(prop, val)
}

fn is_supported_property(name: &str) -> bool {
    matches!(
        name,
        "display"
            | "color"
            | "background"
            | "background-color"
            | "font"
            | "font-size"
            | "font-family"
            | "font-weight"
            | "font-style"
            | "line-height"
            | "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "width"
            | "height"
            | "min-width"
            | "max-width"
            | "min-height"
            | "max-height"
            | "inset"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "border"
            | "border-width"
            | "border-color"
            | "border-radius"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "flex"
            | "flex-direction"
            | "flex-wrap"
            | "flex-grow"
            | "flex-shrink"
            | "flex-basis"
            | "order"
            | "gap"
            | "row-gap"
            | "column-gap"
            | "align-items"
            | "align-self"
            | "align-content"
            | "justify-content"
            | "justify-items"
            | "justify-self"
            | "grid"
            | "grid-area"
            | "grid-row"
            | "grid-column"
            | "grid-row-start"
            | "grid-column-start"
            | "grid-template"
            | "grid-template-columns"
            | "grid-template-rows"
            | "box-sizing"
            | "position"
            | "visibility"
            | "opacity"
            | "text-align"
            | "text-decoration"
            | "overflow"
            | "overflow-x"
            | "overflow-y"
            | "z-index"
            | "float"
            | "clear"
            | "cursor"
            | "vertical-align"
            | "white-space"
            | "word-break"
            | "letter-spacing"
    )
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut cursor = 0;
    let bytes = css.as_bytes();
    while cursor < bytes.len() {
        if cursor + 1 < bytes.len() && bytes[cursor] == b'/' && bytes[cursor + 1] == b'*' {
            if let Some(end) = css[cursor + 2..].find("*/") {
                cursor += 2 + end + 2;
            } else {
                break;
            }
        } else {
            out.push(bytes[cursor] as char);
            cursor += 1;
        }
    }
    out
}

pub fn parse_inline_style(style_attr: &str) -> Vec<Declaration> {
    (*parse_inline_style_shared(style_attr)).clone()
}

pub fn parse_inline_style_shared(style_attr: &str) -> Arc<Vec<Declaration>> {
    let trimmed = style_attr.trim();
    if trimmed.is_empty() {
        static EMPTY_DECLS: OnceLock<Arc<Vec<Declaration>>> = OnceLock::new();
        return EMPTY_DECLS.get_or_init(|| Arc::new(Vec::new())).clone();
    }
    let cache = INLINE_STYLE_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    {
        let guard = cache.read().unwrap();
        if let Some(existing) = guard.get(trimmed) {
            return existing.clone();
        }
    }
    let parsed = Arc::new(parse_declarations(trimmed));
    let mut guard = cache.write().unwrap();
    if guard.len() >= 4096 {
        guard.clear();
    }
    guard.insert(trimmed.to_string(), parsed.clone());
    parsed
}

pub fn parse_declarations(decl_block: &str) -> Vec<Declaration> {
    if decl_block.trim().is_empty() {
        return Vec::new();
    }
    let mut declarations = Vec::new();
    for chunk in split_declarations(decl_block) {
        if chunk.trim().is_empty() {
            continue;
        }
        match parse_declarations_via_lightningcss(chunk) {
            Some(decls) => declarations.extend(decls),
            None => declarations.extend(parse_declarations_legacy(chunk)),
        }
    }
    declarations
}

fn split_declarations(decl_block: &str) -> Vec<&str> {
    let bytes = decl_block.as_bytes();
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(open) = quote {
            if byte == b'\\' {
                index += 2;
                continue;
            }
            if byte == open {
                quote = None;
            }
        } else {
            match byte {
                b'"' | b'\'' => quote = Some(byte),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    match decl_block[index + 2..].find("*/") {
                        Some(end) => index += end + 4,
                        None => break,
                    }
                    continue;
                }
                b';' if depth == 0 => {
                    chunks.push(&decl_block[start..index]);
                    start = index + 1;
                }
                _ => {}
            }
        }
        index += 1;
    }
    chunks.push(&decl_block[start..]);
    chunks
}

fn parse_declarations_via_lightningcss(decl_block: &str) -> Option<Vec<Declaration>> {
    use lightningcss::stylesheet::ParserOptions;

    let wrapped = format!(".x{{{decl_block}}}");
    let options = ParserOptions {
        filename: "inline.css".into(),
        error_recovery: true,
        ..Default::default()
    };
    if let Ok(sheet) = lightningcss::stylesheet::StyleSheet::parse(&wrapped, options) {
        for rule in sheet.rules.0.iter() {
            if let lightningcss::rules::CssRule::Style(style) = rule {
                return Some(declarations_from_block(&style.declarations));
            }
        }
    }
    None
}

fn parse_declarations_legacy(decl_block: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();

    for stmt in split_declarations(decl_block) {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some((name_part, mut val_part)) = trimmed.split_once(':') {
            let trimmed_name = name_part.trim();
            let name = if trimmed_name.starts_with("--") {
                rix_core::InternedStr::from(trimmed_name)
            } else {
                rix_core::InternedStr::from(trimmed_name.to_lowercase())
            };
            val_part = val_part.trim();

            let mut important = false;
            let (val_no_important, is_important) = strip_important(val_part);
            if is_important {
                important = true;
                val_part = val_no_important;
            }

            declarations.push(Declaration {
                name,
                value: val_part.trim().to_string(),
                important,
            });
        }
    }

    declarations
}

fn strip_important(val: &str) -> (&str, bool) {
    let lower = val.to_lowercase();
    if let Some(bang) = lower.rfind('!') {
        let marker = lower[bang..].trim();
        let compact: String = marker.split_whitespace().collect();
        if compact == "!important" {
            return (val[..bang].trim(), true);
        }
    }
    (val, false)
}

pub fn default_user_agent_stylesheet() -> Arc<StyleSheet> {
    static UA_CACHE: std::sync::OnceLock<Arc<StyleSheet>> = std::sync::OnceLock::new();
    UA_CACHE
        .get_or_init(|| {
            let ua_css = r#"
        html { display: block; font-size: 16px; color: #000000; background-color: #ffffff; }
        body { display: block; margin: 8px; font-family: sans-serif; }
        div { display: block; }
        p { display: block; margin-top: 1em; margin-bottom: 1em; }
        h1 { display: block; font-size: 2em; font-weight: bold; margin-top: 0.67em; margin-bottom: 0.67em; }
        h2 { display: block; font-size: 1.5em; font-weight: bold; margin-top: 0.83em; margin-bottom: 0.83em; }
        h3 { display: block; font-size: 1.17em; font-weight: bold; margin-top: 1em; margin-bottom: 1em; }
        h4, h5, h6 { display: block; font-weight: bold; margin-top: 1.33em; margin-bottom: 1.33em; }
        span, a { display: inline; }
        a { color: #0000ee; text-decoration: underline; cursor: pointer; }
        ul, ol { display: block; margin-top: 1em; margin-bottom: 1em; padding-left: 40px; }
        li { display: list-item; }
        header, nav, main, footer, section, article, aside { display: block; }
        pre { display: block; font-family: monospace; white-space: pre; margin-top: 1em; margin-bottom: 1em; }
        table { display: table; }
        tr { display: table-row; }
        td, th { display: table-cell; padding: 1px; }
        b, strong { font-weight: bold; }
        i, em { font-style: italic; }
        form { display: block; }
        center { display: block; text-align: center; }
        input, button, textarea, select {
            display: inline-block;
            font: inherit;
            margin: 0;
            box-sizing: border-box;
        }
        input, textarea {
            text-align: start;
            padding: 3px 6px;
            border: 1px solid #767676;
            border-radius: 3px;
            background-color: #ffffff;
            color: #000000;
            cursor: text;
        }
        input[type=checkbox], input[type=radio] {
            padding: 0;
            border: none;
            background-color: transparent;
            cursor: pointer;
        }
        input[type=reset], input[type=button], input[type=submit], button {
            text-align: center;
            padding: 3px 8px;
            border: 1px solid #767676;
            border-radius: 3px;
            background-color: #efefef;
            color: #000000;
            align-content: center;
            cursor: pointer;
        }
        input[type=color] { align-content: center; }
        svg, img, iframe, video { display: inline-block; }
        iframe { border: 2px inset; }
        head, style, title, script, meta, link, noscript, template { display: none !important; }
        input[type="hidden"], input[type=hidden] { display: none !important; }
        [hidden] { display: none !important; }
    "#;
            StyleSheet::parse(ua_css)
        })
        .clone()
}

#[derive(Debug, Clone, PartialEq)]
pub struct CssomParsedRule {
    pub rule_type: u16,
    pub css_text: String,
    pub selector_text: Option<String>,
    pub declarations: Vec<Declaration>,
    pub condition_text: Option<String>,
    pub children: Vec<CssomParsedRule>,
}

pub fn serialize_declarations(declarations: &[Declaration]) -> String {
    declarations
        .iter()
        .map(|d| {
            format!(
                "{}: {}{};",
                d.name,
                d.value,
                if d.important { " !important" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn lower_lightning_rule_to_cssom(rule: &lightningcss::rules::CssRule) -> Option<CssomParsedRule> {
    use lightningcss::rules::CssRule;
    match rule {
        CssRule::Style(style) => {
            let selector = css_serialize(&style.selectors);
            if selector.trim().is_empty() {
                return None;
            }
            let declarations = declarations_from_block(&style.declarations);
            let decls_str = serialize_declarations(&declarations);
            let css_text = if decls_str.is_empty() {
                format!("{selector} {{}}")
            } else {
                format!("{selector} {{ {decls_str} }}")
            };
            Some(CssomParsedRule {
                rule_type: 1,
                css_text,
                selector_text: Some(selector),
                declarations,
                condition_text: None,
                children: Vec::new(),
            })
        }
        CssRule::Media(media) => {
            let condition = css_serialize(&media.query);
            let mut children = Vec::new();
            for inner in &media.rules.0 {
                if let Some(child) = lower_lightning_rule_to_cssom(inner) {
                    children.push(child);
                }
            }
            let children_text = children
                .iter()
                .map(|c| c.css_text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let css_text = if children_text.is_empty() {
                format!("@media {condition} {{}}")
            } else {
                format!("@media {condition} {{ {children_text} }}")
            };
            Some(CssomParsedRule {
                rule_type: 4,
                css_text,
                selector_text: None,
                declarations: Vec::new(),
                condition_text: Some(condition),
                children,
            })
        }
        CssRule::Supports(supports) => {
            let condition = css_serialize(&supports.condition);
            let mut children = Vec::new();
            for inner in &supports.rules.0 {
                if let Some(child) = lower_lightning_rule_to_cssom(inner) {
                    children.push(child);
                }
            }
            let children_text = children
                .iter()
                .map(|c| c.css_text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let css_text = if children_text.is_empty() {
                format!("@supports {condition} {{}}")
            } else {
                format!("@supports {condition} {{ {children_text} }}")
            };
            Some(CssomParsedRule {
                rule_type: 12,
                css_text,
                selector_text: None,
                declarations: Vec::new(),
                condition_text: Some(condition),
                children,
            })
        }
        CssRule::Import(import) => {
            let css_text = css_serialize(import);
            Some(CssomParsedRule {
                rule_type: 3,
                css_text,
                selector_text: None,
                declarations: Vec::new(),
                condition_text: None,
                children: Vec::new(),
            })
        }
        CssRule::FontFace(font_face) => {
            let css_text = css_serialize(font_face);
            Some(CssomParsedRule {
                rule_type: 5,
                css_text,
                selector_text: None,
                declarations: Vec::new(),
                condition_text: None,
                children: Vec::new(),
            })
        }
        CssRule::Page(page) => {
            let css_text = css_serialize(page);
            Some(CssomParsedRule {
                rule_type: 6,
                css_text,
                selector_text: None,
                declarations: Vec::new(),
                condition_text: None,
                children: Vec::new(),
            })
        }
        CssRule::Keyframes(keyframes) => {
            let css_text = css_serialize(keyframes);
            Some(CssomParsedRule {
                rule_type: 7,
                css_text,
                selector_text: None,
                declarations: Vec::new(),
                condition_text: None,
                children: Vec::new(),
            })
        }
        CssRule::Namespace(ns) => {
            let css_text = css_serialize(ns);
            Some(CssomParsedRule {
                rule_type: 10,
                css_text,
                selector_text: None,
                declarations: Vec::new(),
                condition_text: None,
                children: Vec::new(),
            })
        }
        other => {
            let css_text = css_serialize(other);
            if css_text.trim().is_empty() {
                None
            } else {
                Some(CssomParsedRule {
                    rule_type: 0,
                    css_text,
                    selector_text: None,
                    declarations: Vec::new(),
                    condition_text: None,
                    children: Vec::new(),
                })
            }
        }
    }
}

pub fn parse_cssom_rule(css: &str) -> Result<CssomParsedRule, String> {
    use lightningcss::stylesheet::{ParserOptions, StyleSheet as LightningSheet};

    let trimmed = css.trim();
    if trimmed.is_empty() {
        return Err("Empty rule".into());
    }

    let options = ParserOptions {
        filename: "rule.css".into(),
        error_recovery: false,
        ..Default::default()
    };

    if let Ok(sheet) = LightningSheet::parse(trimmed, options) {
        if sheet.rules.0.len() != 1 {
            return Err("Expected exactly one rule".into());
        }
        if let Some(rule) = lower_lightning_rule_to_cssom(&sheet.rules.0[0]) {
            return Ok(rule);
        }
    }

    let legacy = parse_legacy(trimmed, None, None, None, None);
    if legacy.rules.len() == 1 {
        let r = &legacy.rules[0];
        if let Some((sel_part, _)) = trimmed.split_once('{') {
            let sel = sel_part.trim().to_string();
            if !sel.is_empty() {
                let decls_str = serialize_declarations(&r.declarations);
                let css_text = if decls_str.is_empty() {
                    format!("{sel} {{}}")
                } else {
                    format!("{sel} {{ {decls_str} }}")
                };
                return Ok(CssomParsedRule {
                    rule_type: 1,
                    css_text,
                    selector_text: Some(sel),
                    declarations: r.declarations.clone(),
                    condition_text: None,
                    children: Vec::new(),
                });
            }
        }
    }

    Err("Failed to parse rule".into())
}

pub fn parse_cssom_stylesheet(css: &str) -> Vec<CssomParsedRule> {
    use lightningcss::stylesheet::{ParserOptions, StyleSheet as LightningSheet};

    let trimmed = css.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let options = ParserOptions {
        filename: "style.css".into(),
        error_recovery: true,
        ..Default::default()
    };

    if let Ok(sheet) = LightningSheet::parse(trimmed, options) {
        let mut rules = Vec::new();
        for r in &sheet.rules.0 {
            if let Some(rule) = lower_lightning_rule_to_cssom(r) {
                rules.push(rule);
            }
        }
        if !rules.is_empty() {
            return rules;
        }
    }

    let legacy = parse_legacy(trimmed, None, None, None, None);
    let mut rules = Vec::new();
    for r in &legacy.rules {
        let sel = r
            .selectors
            .iter()
            .map(|s| {
                s.chain
                    .iter()
                    .map(|c| {
                        c.predicates
                            .iter()
                            .map(|p| match p {
                                rix_dom::query::SimpleSelector::Tag(t) => t.clone(),
                                rix_dom::query::SimpleSelector::Class(cl) => format!(".{cl}"),
                                rix_dom::query::SimpleSelector::Id(id) => format!("#{id}"),
                                _ => "*".to_string(),
                            })
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let decls_str = serialize_declarations(&r.declarations);
        let css_text = if decls_str.is_empty() {
            format!("{sel} {{}}")
        } else {
            format!("{sel} {{ {decls_str} }}")
        };
        rules.push(CssomParsedRule {
            rule_type: 1,
            css_text,
            selector_text: Some(sel),
            declarations: r.declarations.clone(),
            condition_text: None,
            children: Vec::new(),
        });
    }
    rules
}
