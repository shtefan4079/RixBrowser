
use super::layout::{host, LayoutSnapshot};
use super::wrap::get_node_id_from_this;
use super::*;
use rix_css::{ComputedStyle, Display};
use rix_dom::{Document, NodeData, NodeId};

enum Item {
    Text { text: String, preserved: bool },
    Break(u32),
    LineFeed,
    Tab,
}

pub(super) fn rendered_text_with_styles(
    document: &Document,
    styles: &impl Fn(NodeId) -> Option<ComputedStyle>,
    root: NodeId,
) -> String {
    let mut items = Vec::new();
    let white_space = white_space_of(styles, root);
    for child in document.children(root) {
        collect_child(document, styles, child, true, true, &white_space, &mut items);
    }
    assemble(items)
}

#[allow(dead_code)]
pub(super) fn rendered_text(
    document: &Document,
    snapshot: &LayoutSnapshot,
    root: NodeId,
) -> String {
    rendered_text_with_styles(document, &|n| snapshot.style(n).cloned(), root)
}

pub(super) fn element_get_inner_text(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return Ok(JsValue::undefined());
    };
    let doc = state.borrow().document.clone();
    let doc_ref = doc.borrow();

    if !shadow_dom::node_is_connected(node, &doc_ref) {
        return Ok(js_string!(doc_ref.inner_text(node)).into());
    }

    if doc_ref.children(node).next().is_none() {
        return Ok(js_string!("").into());
    }

    let host = host(context)?;
    let get_style = |n| host.computed_style_for_node(&doc_ref, n);
    if let Some(style) = get_style(node) {
        if style.display == Display::None {
            return Ok(js_string!("").into());
        }
    }
    let text = rendered_text_with_styles(&doc_ref, &get_style, node);
    Ok(js_string!(text).into())
}

fn collect_child(
    document: &Document,
    styles: &impl Fn(NodeId) -> Option<ComputedStyle>,
    child: NodeId,
    parent_rendered: bool,
    parent_visible: bool,
    parent_white_space: &str,
    items: &mut Vec<Item>,
) {
    if !parent_rendered {
        return;
    }
    match document.get(child) {
        Some(NodeData::Text(text)) => {
            if !parent_visible {
                return;
            }
            if text.chars().any(|character| !character.is_whitespace()) {
                items.push(Item::Text {
                    text: transform_text(text, parent_white_space),
                    preserved: preserves_space(parent_white_space),
                });
            } else if preserves_space(parent_white_space) {
                items.push(Item::Text {
                    text: text.to_string(),
                    preserved: true,
                });
            } else {
                items.push(Item::Text {
                    text: " ".to_string(),
                    preserved: false,
                });
            }
        }
        Some(NodeData::Element(_)) => collect_node(document, styles, child, items),
        _ => {}
    }
}

fn collect_node(
    document: &Document,
    styles: &impl Fn(NodeId) -> Option<ComputedStyle>,
    node: NodeId,
    out: &mut Vec<Item>,
) {
    let name = element_name(document, node);
    let control = matches!(name.as_str(), "select" | "optgroup" | "option");
    let block_control = matches!(name.as_str(), "optgroup" | "option");
    let display = if block_control {
        Some(Display::Block)
    } else {
        styles(node).map(|style| style.display)
    };
    let visible = styles(node)
        .map(|style| style.visibility == "visible")
        .unwrap_or(true);
    let rendered = control || display != Some(Display::None);
    let white_space = white_space_of(styles, node);
    let mut items = Vec::new();
    let replaced = matches!(
        name.as_str(),
        "textarea" | "input" | "video" | "audio" | "canvas" | "iframe" | "embed" | "object" | "img"
    );
    for child in document.children(node) {
        collect_child(
            document,
            styles,
            child,
            rendered && !replaced,
            visible,
            &white_space,
            &mut items,
        );
    }
    if !visible || !rendered {
        out.extend(items);
        return;
    }
    if name == "br" {
        out.push(Item::LineFeed);
        return;
    }
    if display == Some(Display::TableCell) && !is_last_table_cell(document, styles, node) {
        items.push(Item::Tab);
    }
    if display == Some(Display::TableRow) && has_following_table_row(document, styles, node) {
        items.push(Item::LineFeed);
    }
    if name == "p" {
        items.insert(0, Item::Break(2));
        items.push(Item::Break(2));
    }
    if display.map_or(control, |display| display.is_block_level()) {
        items.insert(0, Item::Break(1));
        items.push(Item::Break(1));
    }
    out.extend(items);
}

fn element_name(document: &Document, node: NodeId) -> String {
    match document.get(node) {
        Some(NodeData::Element(element)) => element.name.local.to_string().to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn white_space_of(styles: &impl Fn(NodeId) -> Option<ComputedStyle>, node: NodeId) -> String {
    styles(node)
        .map(|style| style.white_space)
        .unwrap_or_else(|| "normal".to_string())
}

fn is_last_table_cell(
    document: &Document,
    styles: &impl Fn(NodeId) -> Option<ComputedStyle>,
    node: NodeId,
) -> bool {
    let Some(parent) = document.parent(node) else {
        return true;
    };
    let mut after = false;
    for sibling in document.children(parent) {
        if sibling == node {
            after = true;
            continue;
        }
        if !after {
            continue;
        }
        if styles(sibling).map(|style| style.display) == Some(Display::TableCell) {
            return false;
        }
    }
    true
}

fn has_following_table_row(
    document: &Document,
    styles: &impl Fn(NodeId) -> Option<ComputedStyle>,
    node: NodeId,
) -> bool {
    let mut current = node;
    while let Some(parent) = document.parent(current) {
        let mut after = false;
        for sibling in document.children(parent) {
            if sibling == current {
                after = true;
                continue;
            }
            if !after {
                continue;
            }
            if subtree_has_row(document, styles, sibling) {
                return true;
            }
        }
        if element_name(document, parent) == "table" {
            return false;
        }
        current = parent;
    }
    false
}

fn subtree_has_row(
    document: &Document,
    styles: &impl Fn(NodeId) -> Option<ComputedStyle>,
    node: NodeId,
) -> bool {
    if styles(node).map(|style| style.display) == Some(Display::TableRow) {
        return true;
    }
    document
        .children(node)
        .any(|child| subtree_has_row(document, styles, child))
}

fn transform_text(text: &str, white_space: &str) -> String {
    match white_space {
        "pre" | "pre-wrap" | "break-spaces" => text.to_string(),
        "pre-line" => {
            let mut out = String::with_capacity(text.len());
            let mut pending_space = false;
            for character in text.chars() {
                match character {
                    ' ' | '\t' | '\u{c}' => pending_space = true,
                    '\n' => {
                        pending_space = false;
                        out.push('\n');
                    }
                    _ => {
                        if pending_space {
                            out.push(' ');
                            pending_space = false;
                        }
                        out.push(character);
                    }
                }
            }
            out
        }
        _ => {
            let mut out = String::with_capacity(text.len());
            let mut pending_space = false;
            for character in text.chars() {
                if matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
                    pending_space = true;
                } else {
                    if pending_space {
                        out.push(' ');
                        pending_space = false;
                    }
                    out.push(character);
                }
            }
            if pending_space {
                out.push(' ');
            }
            out
        }
    }
}

fn preserves_space(white_space: &str) -> bool {
    matches!(white_space, "pre" | "pre-wrap" | "break-spaces")
}

fn assemble(items: Vec<Item>) -> String {
    let mut out = String::new();
    let mut pending_break: u32 = 0;
    let mut pending_space = false;
    let mut pending_tab = false;
    let flush_space = |out: &mut String, tab: &mut bool, space: &mut bool| {
        if *tab {
            out.push('\t');
        } else if *space {
            out.push(' ');
        }
        *tab = false;
        *space = false;
    };
    for item in items {
        match item {
            Item::Break(count) => {
                pending_space = false;
                pending_tab = false;
                pending_break = pending_break.max(count);
            }
            Item::Tab => {
                if out.is_empty() {
                    pending_break = 0;
                    pending_space = false;
                    pending_tab = false;
                } else if pending_break > 0 {
                    for _ in 0..pending_break {
                        out.push('\n');
                    }
                    pending_break = 0;
                    pending_space = false;
                    pending_tab = false;
                }
                out.push('\t');
            }
            Item::LineFeed => {
                if out.is_empty() {
                    pending_break = 0;
                    pending_space = false;
                    pending_tab = false;
                } else if pending_break > 0 {
                    for _ in 0..pending_break {
                        out.push('\n');
                    }
                    pending_break = 0;
                    pending_space = false;
                    pending_tab = false;
                }
                out.push('\n');
            }
            Item::Text { text, preserved } => {
                if preserved {
                    if !out.is_empty() && pending_break > 0 {
                        for _ in 0..pending_break {
                            out.push('\n');
                        }
                        pending_break = 0;
                    }
                    pending_space = false;
                    pending_tab = false;
                    out.push_str(&text);
                    continue;
                }
                for character in text.chars() {
                    match character {
                        ' ' => pending_space = true,
                        '\t' => pending_tab = true,
                        '\n' => {
                            pending_space = false;
                            pending_tab = false;
                            pending_break = pending_break.max(1);
                        }
                        _ => {
                            if out.is_empty() {
                                pending_break = 0;
                                pending_space = false;
                                pending_tab = false;
                            } else if pending_break > 0 {
                                for _ in 0..pending_break {
                                    out.push('\n');
                                }
                                pending_break = 0;
                                pending_space = false;
                                pending_tab = false;
                            }
                            flush_space(&mut out, &mut pending_tab, &mut pending_space);
                            out.push(character);
                        }
                    }
                }
            }
        }
    }
    out
}
