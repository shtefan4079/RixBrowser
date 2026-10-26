
use crate::node::{ElementData, NodeData};
use crate::tree::Document;
use indextree::NodeId;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

static PARSED_SELECTORS_CACHE: OnceLock<
    RwLock<HashMap<String, Result<Arc<Vec<ComplexSelector>>, String>>>,
> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NthExpr {
    pub a: i64,
    pub b: i64,
}

impl NthExpr {
    pub fn matches(self, index: usize) -> bool {
        let index = index as i64;
        if self.a == 0 {
            return index == self.b;
        }
        let diff = index - self.b;
        diff % self.a == 0 && diff / self.a >= 0
    }

    pub fn parse(input: &str) -> Result<Self, String> {
        let text: String = input
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        if text.is_empty() {
            return Err("Empty An+B expression".into());
        }
        if text == "odd" {
            return Ok(Self { a: 2, b: 1 });
        }
        if text == "even" {
            return Ok(Self { a: 2, b: 0 });
        }
        let Some(n_pos) = text.find('n') else {
            let b = text
                .parse::<i64>()
                .map_err(|_| format!("Invalid An+B expression: {input}"))?;
            return Ok(Self { a: 0, b });
        };
        let (a_part, b_part) = (&text[..n_pos], &text[n_pos + 1..]);
        let a = match a_part {
            "" | "+" => 1,
            "-" => -1,
            other => other
                .parse::<i64>()
                .map_err(|_| format!("Invalid An+B coefficient: {input}"))?,
        };
        let b = if b_part.is_empty() {
            0
        } else {
            b_part
                .parse::<i64>()
                .map_err(|_| format!("Invalid An+B constant: {input}"))?
        };
        Ok(Self { a, b })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PseudoClass {
    Root,
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
    Disabled,
    FirstChild,
    LastChild,
    OnlyChild,
    Empty,
    NthChild(NthExpr),
    NthLastChild(NthExpr),
    FirstOfType,
    LastOfType,
    OnlyOfType,
    NthOfType(NthExpr),
    NthLastOfType(NthExpr),
    Not(Box<SimpleSelector>),
    Is(Vec<ComplexSelector>),
    Where(Vec<ComplexSelector>),
    Host(Option<Box<CompoundSelector>>),
    Slotted(Box<CompoundSelector>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeCase {
    Default,
    AsciiCaseInsensitive,
    CaseSensitive,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SimpleSelector {
    Universal,
    Tag(String),
    Id(String),
    Class(String),
    AttributeExact(String, String, AttributeCase),
    AttributePrefix(String, String, AttributeCase),
    AttributeSuffix(String, String, AttributeCase),
    AttributeContains(String, String, AttributeCase),
    AttributeWord(String, String, AttributeCase),
    AttributeDashMatch(String, String, AttributeCase),
    AttributeExists(String),
    Pseudo(PseudoClass),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    AdjacentSibling,
    GeneralSibling,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompoundSelector {
    pub predicates: Vec<SimpleSelector>,
    pub combinator_to_next: Option<Combinator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PseudoElement {
    Before,
    After,
    Placeholder,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComplexSelector {
    pub chain: Vec<CompoundSelector>,
    pub pseudo_element: Option<PseudoElement>,
}

fn split_selector_list(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut start = 0usize;
    for (index, ch) in input.char_indices() {
        match quote {
            Some(open) => {
                if ch == open {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                '(' | '[' => depth += 1,
                ')' | ']' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    parts.push(&input[start..index]);
                    start = index + ch.len_utf8();
                }
                _ => {}
            },
        }
    }
    parts.push(&input[start..]);
    parts.into_iter().filter(|part| !part.trim().is_empty()).collect()
}

impl ComplexSelector {
    pub fn parse(input: &str) -> Result<Vec<Self>, String> {
        Self::parse_shared(input).map(|s| (*s).clone())
    }

    pub fn parse_shared(input: &str) -> Result<Arc<Vec<Self>>, String> {
        let trimmed = input.trim();
        let cache = PARSED_SELECTORS_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
        {
            let guard = cache.read().unwrap();
            if let Some(res) = guard.get(trimmed) {
                return res.clone();
            }
        }
        let parsed = Self::parse_uncached(trimmed);
        let mut guard = cache.write().unwrap();
        if guard.len() >= 4096 {
            guard.clear();
        }
        match parsed {
            Ok(v) => {
                let shared = Arc::new(v);
                guard.insert(trimmed.to_string(), Ok(shared.clone()));
                Ok(shared)
            }
            Err(e) => {
                guard.insert(trimmed.to_string(), Err(e.clone()));
                Err(e)
            }
        }
    }

    fn parse_uncached(input: &str) -> Result<Vec<Self>, String> {
        let mut selectors = Vec::new();
        for part in split_selector_list(input) {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(s) = Self::parse_single(trimmed) {
                selectors.push(s);
            }
        }
        if selectors.is_empty() {
            return Err("Empty selector".into());
        }
        Ok(selectors)
    }

    fn parse_single(input: &str) -> Result<Self, String> {
        let mut chain = Vec::new();
        let tokens = tokenize_selector(input)?;

        let mut current_predicates = Vec::new();
        let mut pseudo_element: Option<PseudoElement> = None;

        for token in tokens {
            match token {
                SelectorToken::Combinator(c) => {
                    if pseudo_element.is_some() {
                        return Err("Pseudo-element cannot be followed by combinators".into());
                    }
                    if !current_predicates.is_empty() {
                        chain.push(CompoundSelector {
                            predicates: std::mem::take(&mut current_predicates),
                            combinator_to_next: Some(c),
                        });
                    }
                }
                SelectorToken::Simple(s) => {
                    if pseudo_element.is_some() {
                        return Err("Pseudo-element cannot be followed by simple selectors".into());
                    }
                    current_predicates.push(s);
                }
                SelectorToken::PseudoElement(pseudo) => {
                    if pseudo_element.is_some() {
                        return Err("Selector has more than one pseudo-element".into());
                    }
                    pseudo_element = Some(pseudo);
                }
            }
        }

        if !current_predicates.is_empty() {
            chain.push(CompoundSelector {
                predicates: current_predicates,
                combinator_to_next: None,
            });
        }

        if chain.is_empty() {
            return Err("Invalid empty selector chain".into());
        }

        for (idx, compound) in chain.iter().enumerate() {
            if idx + 1 < chain.len()
                && compound.predicates.iter().any(|p| {
                    matches!(p, SimpleSelector::Pseudo(PseudoClass::Slotted(_)))
                })
            {
                return Err("::slotted() pseudo-element cannot be followed by combinators".into());
            }
        }

        Ok(Self {
            chain,
            pseudo_element,
        })
    }
}

enum SelectorToken {
    Simple(SimpleSelector),
    Combinator(Combinator),
    PseudoElement(PseudoElement),
}

fn tokenize_selector(input: &str) -> Result<Vec<SelectorToken>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if c.is_whitespace() {
            let mut j = i;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() {
                if chars[j] == '>' {
                    tokens.push(SelectorToken::Combinator(Combinator::Child));
                    i = j + 1;
                    continue;
                } else if chars[j] == '+' {
                    tokens.push(SelectorToken::Combinator(Combinator::AdjacentSibling));
                    i = j + 1;
                    continue;
                } else if chars[j] == '~' {
                    tokens.push(SelectorToken::Combinator(Combinator::GeneralSibling));
                    i = j + 1;
                    continue;
                }
            }
            tokens.push(SelectorToken::Combinator(Combinator::Descendant));
            i = j;
            continue;
        }

        if c == '>' {
            tokens.push(SelectorToken::Combinator(Combinator::Child));
            i += 1;
            continue;
        }

        if c == '+' {
            tokens.push(SelectorToken::Combinator(Combinator::AdjacentSibling));
            i += 1;
            continue;
        }

        if c == '~' {
            tokens.push(SelectorToken::Combinator(Combinator::GeneralSibling));
            i += 1;
            continue;
        }

        if c == '*' {
            tokens.push(SelectorToken::Simple(SimpleSelector::Universal));
            i += 1;
            continue;
        }

        if c == ':' {
            let is_double_colon = i + 1 < chars.len() && chars[i + 1] == ':';
            if is_double_colon {
                i += 2;
            } else {
                i += 1;
            }
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '-') {
                i += 1;
            }
            if i < chars.len() && chars[i] == '(' {
                let mut depth = 0;
                while i < chars.len() {
                    if chars[i] == '(' {
                        depth += 1;
                    }
                    if chars[i] == ')' {
                        depth -= 1;
                    }
                    i += 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            let pseudo_str: String = chars[start..i].iter().collect();
            let pseudo_name = pseudo_str.to_ascii_lowercase();
            match pseudo_name.as_str() {
                "before" => {
                    tokens.push(SelectorToken::PseudoElement(PseudoElement::Before));
                    continue;
                }
                "after" => {
                    tokens.push(SelectorToken::PseudoElement(PseudoElement::After));
                    continue;
                }
                "placeholder" => {
                    tokens.push(SelectorToken::PseudoElement(PseudoElement::Placeholder));
                    continue;
                }
                _ => {}
            }
            tokens.push(SelectorToken::Simple(SimpleSelector::Pseudo(
                parse_pseudo_class(&pseudo_str)?,
            )));
            continue;
        }

        if c == '#' {
            i += 1;
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '_')
            {
                i += 1;
            }
            let id: String = chars[start..i].iter().collect();
            tokens.push(SelectorToken::Simple(SimpleSelector::Id(id)));
            continue;
        }

        if c == '.' {
            i += 1;
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '_')
            {
                i += 1;
            }
            let class: String = chars[start..i].iter().collect();
            tokens.push(SelectorToken::Simple(SimpleSelector::Class(class)));
            continue;
        }

        if c == '[' {
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != ']' {
                i += 1;
            }
            if i >= chars.len() {
                return Err("Unclosed attribute selector".into());
            }
            let attr_expr: String = chars[start..i].iter().collect();
            i += 1;

            tokens.push(SelectorToken::Simple(parse_attribute_selector(&attr_expr)?));
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '_')
            {
                i += 1;
            }
            let tag: String = chars[start..i].iter().collect();
            tokens.push(SelectorToken::Simple(SimpleSelector::Tag(
                tag.to_lowercase(),
            )));
            continue;
        }

        i += 1;
    }

    Ok(tokens)
}

fn parse_pseudo_class(s: &str) -> Result<PseudoClass, String> {
    match s {
        "root" => Ok(PseudoClass::Root),
        "hover" => Ok(PseudoClass::Hover),
        "active" => Ok(PseudoClass::Active),
        "focus" => Ok(PseudoClass::Focus),
        "focus-visible" => Ok(PseudoClass::FocusVisible),
        "focus-within" => Ok(PseudoClass::FocusWithin),
        "disabled" => Ok(PseudoClass::Disabled),
        other if other.starts_with("is(") && other.ends_with(')') => {
            Ok(PseudoClass::Is(ComplexSelector::parse(
                &other[3..other.len() - 1],
            )?))
        }
        other if other.starts_with("where(") && other.ends_with(')') => {
            Ok(PseudoClass::Where(ComplexSelector::parse(
                &other[6..other.len() - 1],
            )?))
        }
        other if other.starts_with("matches(") && other.ends_with(')') => {
            Ok(PseudoClass::Is(ComplexSelector::parse(
                &other[8..other.len() - 1],
            )?))
        }
        other if other.starts_with("not(") && other.ends_with(')') => {
            let selectors = ComplexSelector::parse(&other[4..other.len() - 1])?;
            if selectors.len() == 1
                && selectors[0].chain.len() == 1
                && selectors[0].chain[0].predicates.len() == 1
            {
                Ok(PseudoClass::Not(Box::new(
                    selectors[0].chain[0].predicates[0].clone(),
                )))
            } else {
                Err("Only single-predicate :not() is supported".into())
            }
        }
        "first-child" => Ok(PseudoClass::FirstChild),
        "last-child" => Ok(PseudoClass::LastChild),
        "only-child" => Ok(PseudoClass::OnlyChild),
        "empty" => Ok(PseudoClass::Empty),
        "host" => Ok(PseudoClass::Host(None)),
        other if other.starts_with("host(") && other.ends_with(')') => {
            let inner = other[5..other.len() - 1].trim();
            if inner.is_empty() {
                return Err("Empty :host() selector".into());
            }
            let tokens = tokenize_selector(inner)?;
            let mut predicates = Vec::new();
            for token in tokens {
                match token {
                    SelectorToken::Simple(s) => predicates.push(s),
                    SelectorToken::PseudoElement(_) => {
                        return Err(format!(
                            ":host() argument cannot contain pseudo-elements: {inner}"
                        ));
                    }
                    SelectorToken::Combinator(_) => {
                        return Err(format!(
                            ":host() argument cannot contain combinators: {inner}"
                        ));
                    }
                }
            }
            if predicates.is_empty() {
                return Err(format!("Empty :host() selector: {inner}"));
            }
            Ok(PseudoClass::Host(Some(Box::new(CompoundSelector {
                predicates,
                combinator_to_next: None,
            }))))
        }
        other if other.starts_with("slotted(") && other.ends_with(')') => {
            let inner = other[8..other.len() - 1].trim();
            if inner.is_empty() {
                return Err("Empty ::slotted() selector".into());
            }
            let tokens = tokenize_selector(inner)?;
            let mut predicates = Vec::new();
            for token in tokens {
                match token {
                    SelectorToken::Simple(s) => predicates.push(s),
                    SelectorToken::PseudoElement(_) => {
                        return Err(format!(
                            "::slotted() argument cannot contain pseudo-elements: {inner}"
                        ));
                    }
                    SelectorToken::Combinator(_) => {
                        return Err(format!(
                            "::slotted() argument cannot contain combinators: {inner}"
                        ));
                    }
                }
            }
            if predicates.is_empty() {
                return Err(format!("Empty ::slotted() selector: {inner}"));
            }
            Ok(PseudoClass::Slotted(Box::new(CompoundSelector {
                predicates,
                combinator_to_next: None,
            })))
        }
        "first-of-type" => Ok(PseudoClass::FirstOfType),
        "last-of-type" => Ok(PseudoClass::LastOfType),
        "only-of-type" => Ok(PseudoClass::OnlyOfType),
        other if other.starts_with("nth-child(") && other.ends_with(')') => {
            Ok(PseudoClass::NthChild(nth_argument(other, "nth-child")?))
        }
        other if other.starts_with("nth-last-child(") && other.ends_with(')') => Ok(
            PseudoClass::NthLastChild(nth_argument(other, "nth-last-child")?),
        ),
        other if other.starts_with("nth-of-type(") && other.ends_with(')') => {
            Ok(PseudoClass::NthOfType(nth_argument(other, "nth-of-type")?))
        }
        other if other.starts_with("nth-last-of-type(") && other.ends_with(')') => Ok(
            PseudoClass::NthLastOfType(nth_argument(other, "nth-last-of-type")?),
        ),
        other => Err(format!("Unsupported pseudo-class: :{other}")),
    }
}

fn nth_argument(text: &str, name: &str) -> Result<NthExpr, String> {
    let inner = text[name.len() + 1..text.len() - 1].trim();
    NthExpr::parse(inner).map_err(|error| format!("Invalid :{name}(): {error}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttributeOp {
    Exact,
    Prefix,
    Suffix,
    Contains,
    Word,
    DashMatch,
}

fn parse_attribute_selector(expr: &str) -> Result<SimpleSelector, String> {
    let text = expr.trim();
    let name_end = text
        .char_indices()
        .find(|(_, c)| c.is_whitespace() || matches!(c, '~' | '|' | '^' | '$' | '*' | '='))
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let name = &text[..name_end];
    if !is_css_ident(name) {
        return Err(format!("Invalid attribute selector name: [{expr}]"));
    }
    let rest = text[name_end..].trim_start();
    if rest.is_empty() {
        return Ok(SimpleSelector::AttributeExists(name.to_string()));
    }

    let (op, rest) = if let Some(rest) = rest.strip_prefix("~=") {
        (AttributeOp::Word, rest)
    } else if let Some(rest) = rest.strip_prefix("|=") {
        (AttributeOp::DashMatch, rest)
    } else if let Some(rest) = rest.strip_prefix("^=") {
        (AttributeOp::Prefix, rest)
    } else if let Some(rest) = rest.strip_prefix("$=") {
        (AttributeOp::Suffix, rest)
    } else if let Some(rest) = rest.strip_prefix("*=") {
        (AttributeOp::Contains, rest)
    } else if let Some(rest) = rest.strip_prefix('=') {
        (AttributeOp::Exact, rest)
    } else {
        return Err(format!("Invalid attribute selector operator: [{expr}]"));
    };

    let (value, rest) = parse_attribute_value(rest.trim_start())?;
    let flag = rest.trim();
    let case = if flag.is_empty() {
        AttributeCase::Default
    } else if flag.eq_ignore_ascii_case("i") {
        AttributeCase::AsciiCaseInsensitive
    } else if flag.eq_ignore_ascii_case("s") {
        AttributeCase::CaseSensitive
    } else {
        return Err(format!("Invalid attribute selector flag: {flag}"));
    };

    let name = name.to_string();
    Ok(match op {
        AttributeOp::Exact => SimpleSelector::AttributeExact(name, value, case),
        AttributeOp::Prefix => SimpleSelector::AttributePrefix(name, value, case),
        AttributeOp::Suffix => SimpleSelector::AttributeSuffix(name, value, case),
        AttributeOp::Contains => SimpleSelector::AttributeContains(name, value, case),
        AttributeOp::Word => SimpleSelector::AttributeWord(name, value, case),
        AttributeOp::DashMatch => SimpleSelector::AttributeDashMatch(name, value, case),
    })
}

fn parse_attribute_value(input: &str) -> Result<(String, &str), String> {
    let Some(quote @ ('"' | '\'')) = input.chars().next() else {
        let end = input.find(char::is_whitespace).unwrap_or(input.len());
        let token = &input[..end];
        if !is_css_ident(token) {
            return Err(format!("Invalid attribute selector value: {input}"));
        }
        return Ok((token.to_string(), &input[end..]));
    };
    for (offset, ch) in input[1..].char_indices() {
        if ch == quote {
            let rest = &input[1 + offset + ch.len_utf8()..];
            return Ok((input[1..1 + offset].to_string(), rest));
        }
    }
    Err("Unclosed attribute selector value".into())
}

fn is_css_ident(token: &str) -> bool {
    let mut chars = token.chars().peekable();
    let Some(first) = chars.next() else {
        return false;
    };
    let ident_start = |c: char| c.is_ascii_alphabetic() || c == '_' || !c.is_ascii();
    let starts_ident = ident_start(first)
        || (first == '-'
            && matches!(chars.peek(), Some(&next) if ident_start(next) || next == '-'));
    starts_ident && chars.all(|c| ident_start(c) || c.is_ascii_digit() || c == '-')
}

const HTML_CASE_INSENSITIVE_ATTRIBUTE_NAMES: &[&str] = &[
    "accept",
    "accept-charset",
    "align",
    "alink",
    "axis",
    "bgcolor",
    "charset",
    "checked",
    "clear",
    "codetype",
    "color",
    "compact",
    "declare",
    "defer",
    "dir",
    "direction",
    "disabled",
    "enctype",
    "face",
    "frame",
    "hreflang",
    "http-equiv",
    "lang",
    "language",
    "link",
    "media",
    "method",
    "multiple",
    "nohref",
    "noresize",
    "noshade",
    "nowrap",
    "readonly",
    "rel",
    "rev",
    "rules",
    "scope",
    "scrolling",
    "selected",
    "shape",
    "target",
    "text",
    "type",
    "valign",
    "valuetype",
    "vlink",
];

fn is_html_element(elem: &ElementData) -> bool {
    &*elem.name.ns == "http://www.w3.org/1999/xhtml"
}

fn lookup_attribute<'a>(elem: &'a ElementData, attr: &str) -> Option<&'a str> {
    if is_html_element(elem) {
        elem.attributes
            .iter()
            .find(|(name, _)| name.ns.is_empty() && (&*name.local).eq_ignore_ascii_case(attr))
            .map(|(_, value)| value.as_str())
    } else {
        elem.get_attribute(attr)
    }
}

fn attribute_case_insensitive(case: AttributeCase, elem: &ElementData, attr: &str) -> bool {
    match case {
        AttributeCase::AsciiCaseInsensitive => true,
        AttributeCase::CaseSensitive => false,
        AttributeCase::Default => {
            is_html_element(elem)
                && HTML_CASE_INSENSITIVE_ATTRIBUTE_NAMES
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(attr))
        }
    }
}

fn attribute_value_matches(
    elem: &ElementData,
    attr: &str,
    val: &str,
    case: AttributeCase,
    op: AttributeOp,
) -> bool {
    let Some(value) = lookup_attribute(elem, attr) else {
        return false;
    };
    let insensitive = attribute_case_insensitive(case, elem, attr);
    match op {
        AttributeOp::Exact => cmp_eq(value, val, insensitive),
        AttributeOp::Prefix => !val.is_empty() && cmp_starts_with(value, val, insensitive),
        AttributeOp::Suffix => !val.is_empty() && cmp_ends_with(value, val, insensitive),
        AttributeOp::Contains => !val.is_empty() && cmp_contains(value, val, insensitive),
        AttributeOp::Word => {
            !val.is_empty()
                && value
                    .split_ascii_whitespace()
                    .any(|word| cmp_eq(word, val, insensitive))
        }
        AttributeOp::DashMatch => {
            cmp_eq(value, val, insensitive)
                || (cmp_starts_with(value, val, insensitive)
                    && value.as_bytes().get(val.len()) == Some(&b'-'))
        }
    }
}

fn cmp_eq(a: &str, b: &str, insensitive: bool) -> bool {
    if insensitive {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn cmp_starts_with(value: &str, prefix: &str, insensitive: bool) -> bool {
    if insensitive {
        let (value, prefix) = (value.as_bytes(), prefix.as_bytes());
        value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix)
    } else {
        value.starts_with(prefix)
    }
}

fn cmp_ends_with(value: &str, suffix: &str, insensitive: bool) -> bool {
    if insensitive {
        let (value, suffix) = (value.as_bytes(), suffix.as_bytes());
        value.len() >= suffix.len()
            && value[value.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
    } else {
        value.ends_with(suffix)
    }
}

fn cmp_contains(value: &str, needle: &str, insensitive: bool) -> bool {
    if insensitive {
        let (value, needle) = (value.as_bytes(), needle.as_bytes());
        needle.len() <= value.len()
            && (0..=value.len() - needle.len())
                .any(|start| value[start..start + needle.len()].eq_ignore_ascii_case(needle))
    } else {
        value.contains(needle)
    }
}

pub fn matches_compound(doc: &Document, node: NodeId, predicates: &[SimpleSelector]) -> bool {
    let elem = match doc.get(node) {
        Some(NodeData::Element(e)) => e,
        _ => return false,
    };

    for pred in predicates {
        match pred {
            SimpleSelector::Universal => {}
            SimpleSelector::Tag(t) => {
                if !(&*elem.name.local).eq_ignore_ascii_case(t) {
                    return false;
                }
            }
            SimpleSelector::Id(id) => {
                if elem.id() != Some(id) {
                    return false;
                }
            }
            SimpleSelector::Class(cls) => {
                if !elem.has_class(cls) {
                    return false;
                }
            }
            SimpleSelector::AttributeExists(attr) => {
                if lookup_attribute(elem, attr).is_none() {
                    return false;
                }
            }
            SimpleSelector::AttributeExact(attr, val, case) => {
                if !attribute_value_matches(elem, attr, val, *case, AttributeOp::Exact) {
                    return false;
                }
            }
            SimpleSelector::AttributePrefix(attr, val, case) => {
                if !attribute_value_matches(elem, attr, val, *case, AttributeOp::Prefix) {
                    return false;
                }
            }
            SimpleSelector::AttributeSuffix(attr, val, case) => {
                if !attribute_value_matches(elem, attr, val, *case, AttributeOp::Suffix) {
                    return false;
                }
            }
            SimpleSelector::AttributeContains(attr, val, case) => {
                if !attribute_value_matches(elem, attr, val, *case, AttributeOp::Contains) {
                    return false;
                }
            }
            SimpleSelector::AttributeWord(attr, val, case) => {
                if !attribute_value_matches(elem, attr, val, *case, AttributeOp::Word) {
                    return false;
                }
            }
            SimpleSelector::AttributeDashMatch(attr, val, case) => {
                if !attribute_value_matches(elem, attr, val, *case, AttributeOp::DashMatch) {
                    return false;
                }
            }
            SimpleSelector::Pseudo(pseudo) => match pseudo {
                PseudoClass::Hover => {
                    if !doc.is_hovered(node) {
                        return false;
                    }
                }
                PseudoClass::Active => {
                    if !doc.is_active(node) {
                        return false;
                    }
                }
                PseudoClass::Focus => {
                    if !doc.is_focused(node) {
                        return false;
                    }
                }
                PseudoClass::FocusVisible => {
                    if !doc.is_focus_visible(node) {
                        return false;
                    }
                }
                PseudoClass::FocusWithin => {
                    if !doc.is_focus_within(node) {
                        return false;
                    }
                }
                PseudoClass::Disabled => {
                    if !matches!(
                        elem.name.local.as_str(),
                        "button"
                            | "input"
                            | "select"
                            | "textarea"
                            | "option"
                            | "optgroup"
                            | "fieldset"
                    ) || elem.get_attribute("disabled").is_none()
                    {
                        return false;
                    }
                }
                PseudoClass::Root => {
                    let parent = match doc.parent(node) {
                        Some(p) => p,
                        None => return false,
                    };
                    if parent != doc.root() {
                        return false;
                    }
                }
                PseudoClass::FirstChild => {
                    let parent = match doc.parent(node) {
                        Some(p) => p,
                        None => return false,
                    };
                    let first_elem_child = doc
                        .children(parent)
                        .find(|&c| matches!(doc.get(c), Some(NodeData::Element(_))));
                    if first_elem_child != Some(node) {
                        return false;
                    }
                }
                PseudoClass::LastChild => {
                    let parent = match doc.parent(node) {
                        Some(p) => p,
                        None => return false,
                    };
                    let last_elem_child = doc
                        .children(parent)
                        .filter(|&c| matches!(doc.get(c), Some(NodeData::Element(_))))
                        .last();
                    if last_elem_child != Some(node) {
                        return false;
                    }
                }
                PseudoClass::OnlyChild => {
                    let parent = match doc.parent(node) {
                        Some(p) => p,
                        None => return false,
                    };
                    let count = doc
                        .children(parent)
                        .filter(|&c| matches!(doc.get(c), Some(NodeData::Element(_))))
                        .count();
                    if count != 1 {
                        return false;
                    }
                }
                PseudoClass::Empty => {
                    if doc.children(node).next().is_some() {
                        return false;
                    }
                }
                PseudoClass::NthChild(expr) => {
                    let parent = match doc.parent(node) {
                        Some(p) => p,
                        None => return false,
                    };
                    let elem_idx = doc
                        .children(parent)
                        .filter(|&c| matches!(doc.get(c), Some(NodeData::Element(_))))
                        .position(|c| c == node);
                    if elem_idx.map(|i| expr.matches(i + 1)) != Some(true) {
                        return false;
                    }
                }
                PseudoClass::NthLastChild(expr) => {
                    let parent = match doc.parent(node) {
                        Some(p) => p,
                        None => return false,
                    };
                    let elem_pos = doc
                        .children(parent)
                        .filter(|&c| matches!(doc.get(c), Some(NodeData::Element(_))))
                        .position(|c| c == node);
                    let total = doc
                        .children(parent)
                        .filter(|&c| matches!(doc.get(c), Some(NodeData::Element(_))))
                        .count();
                    if elem_pos.map(|i| expr.matches(total - i)) != Some(true) {
                        return false;
                    }
                }
                PseudoClass::FirstOfType => {
                    if type_position(doc, node).map(|(index, _)| index) != Some(1) {
                        return false;
                    }
                }
                PseudoClass::LastOfType => {
                    if type_position(doc, node).map(|(index, count)| index == count && count > 0)
                        != Some(true)
                    {
                        return false;
                    }
                }
                PseudoClass::OnlyOfType => {
                    if type_position(doc, node).map(|(_, count)| count) != Some(1) {
                        return false;
                    }
                }
                PseudoClass::NthOfType(expr) => {
                    if type_position(doc, node).map(|(index, _)| expr.matches(index)) != Some(true) {
                        return false;
                    }
                }
                PseudoClass::NthLastOfType(expr) => {
                    if type_position(doc, node).map(|(index, count)| expr.matches(count - index + 1))
                        != Some(true)
                    {
                        return false;
                    }
                }
                PseudoClass::Not(sub_pred) => {
                    if matches_compound(doc, node, std::slice::from_ref(sub_pred.as_ref())) {
                        return false;
                    }
                }
                PseudoClass::Is(list) | PseudoClass::Where(list) => {
                    if !list
                        .iter()
                        .any(|selector| matches_complex(doc, node, selector))
                    {
                        return false;
                    }
                }
                PseudoClass::Host(inner) => {
                    if elem.shadow_root.is_none() {
                        return false;
                    }
                    if let Some(compound) = inner {
                        if !matches_compound(doc, node, &compound.predicates) {
                            return false;
                        }
                    }
                }
                PseudoClass::Slotted(inner) => {
                    if doc.assigned_slot_for(node).is_none() {
                        return false;
                    }
                    if !matches_compound(doc, node, &inner.predicates) {
                        return false;
                    }
                }
            },
        }
    }

    true
}

fn effective_parent(doc: &Document, node: NodeId) -> Option<NodeId> {
    let parent = doc.parent(node)?;
    match doc.get(parent) {
        Some(NodeData::Element(_)) => Some(parent),
        Some(NodeData::ShadowRoot { host, .. }) => Some(*host),
        _ => None,
    }
}

fn type_position(doc: &Document, node: NodeId) -> Option<(usize, usize)> {
    let parent = doc.parent(node)?;
    let Some(NodeData::Element(target)) = doc.get(node) else {
        return None;
    };
    let name = &target.name.local;
    let mut index = 0;
    let mut count = 0;
    for child in doc.children(parent) {
        let Some(NodeData::Element(elem)) = doc.get(child) else {
            continue;
        };
        if elem.name.local.eq_ignore_ascii_case(name) {
            count += 1;
            if child == node {
                index = count;
            }
        }
    }
    Some((index, count))
}

pub fn matches_complex(doc: &Document, node: NodeId, selector: &ComplexSelector) -> bool {
    if selector.chain.is_empty() {
        return false;
    }

    let mut chain_idx = selector.chain.len() - 1;
    let last = &selector.chain[chain_idx];

    if !matches_compound(doc, node, &last.predicates) {
        return false;
    }

    let mut current_node = node;

    while chain_idx > 0 {
        let combinator = selector.chain[chain_idx - 1]
            .combinator_to_next
            .unwrap_or(Combinator::Descendant);
        chain_idx -= 1;
        let expected = &selector.chain[chain_idx];

        match combinator {
            Combinator::Child => {
                let parent = match effective_parent(doc, current_node) {
                    Some(p) => p,
                    None => return false,
                };
                if !matches_compound(doc, parent, &expected.predicates) {
                    return false;
                }
                current_node = parent;
            }
            Combinator::Descendant => {
                let mut found = false;
                let mut curr = current_node;
                while let Some(parent) = effective_parent(doc, curr) {
                    if matches_compound(doc, parent, &expected.predicates) {
                        current_node = parent;
                        found = true;
                        break;
                    }
                    if matches!(
                        doc.parent(curr).and_then(|p| doc.get(p)),
                        Some(NodeData::ShadowRoot { .. })
                    ) {
                        break;
                    }
                    curr = parent;
                }
                if !found {
                    return false;
                }
            }
            Combinator::AdjacentSibling => {
                let parent = match doc.parent(current_node) {
                    Some(p) => p,
                    None => return false,
                };
                let prev_sibling = doc
                    .children(parent)
                    .filter(|&c| matches!(doc.get(c), Some(NodeData::Element(_))))
                    .take_while(|&c| c != current_node)
                    .last();
                match prev_sibling {
                    Some(prev) if matches_compound(doc, prev, &expected.predicates) => {
                        current_node = prev;
                    }
                    _ => return false,
                }
            }
            Combinator::GeneralSibling => {
                let parent = match doc.parent(current_node) {
                    Some(p) => p,
                    None => return false,
                };
                let mut found = false;
                for sibling in doc
                    .children(parent)
                    .filter(|&c| matches!(doc.get(c), Some(NodeData::Element(_))))
                    .take_while(|&c| c != current_node)
                {
                    if matches_compound(doc, sibling, &expected.predicates) {
                        current_node = sibling;
                        found = true;
                        break;
                    }
                }
                if !found {
                    return false;
                }
            }
        }
    }

    true
}

impl Document {
    pub fn query_selector(&self, root: NodeId, selector_str: &str) -> Option<NodeId> {
        let trimmed = selector_str.trim();
        if trimmed.starts_with('#')
            && !trimmed.contains(' ')
            && !trimmed.contains('>')
            && !trimmed.contains('+')
            && !trimmed.contains('~')
            && !trimmed.contains('[')
            && !trimmed.contains(':')
            && !trimmed.contains('.')
        {
            let id = &trimmed[1..];
            if root == self.root() {
                return self.get_element_by_id(id);
            } else if let Some(target) = self.get_element_by_id(id) {
                let mut cur = Some(target);
                while let Some(node) = cur {
                    if node == root {
                        return Some(target);
                    }
                    cur = self.parent(node);
                }
                return None;
            }
        }
        let selectors = ComplexSelector::parse_shared(selector_str).ok()?;
        for desc in self.descendants(root) {
            for sel in selectors.iter() {
                if matches_complex(self, desc, sel) {
                    return Some(desc);
                }
            }
        }
        None
    }

    pub fn query_selector_all(&self, root: NodeId, selector_str: &str) -> Vec<NodeId> {
        let selectors = match ComplexSelector::parse_shared(selector_str) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let mut matches = Vec::new();
        for desc in self.descendants(root) {
            for sel in selectors.iter() {
                if matches_complex(self, desc, sel) {
                    matches.push(desc);
                    break;
                }
            }
        }
        matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complex_selector_caching_and_reuse() {
        let sel1 = ComplexSelector::parse_shared("div.content > p:first-child").unwrap();
        let sel2 = ComplexSelector::parse_shared("div.content > p:first-child").unwrap();
        assert!(Arc::ptr_eq(&sel1, &sel2));

        let res = ComplexSelector::parse("div.content > p:first-child").unwrap();
        assert_eq!(res.len(), 1);
    }
}
