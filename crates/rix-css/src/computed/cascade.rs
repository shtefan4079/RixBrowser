use super::*;

struct CascadeEntry<'a> {
    name: &'a str,
    value: &'a str,
    specificity: Specificity,
    important: bool,
    origin: Origin,
    layer_rank: usize,
    important_layer_rank: usize,
    order: usize,
}

fn push_rule<'a>(
    rule: &'a crate::rule_index::IndexedRule,
    pseudo: Option<PseudoElement>,
    origin: Origin,
    out: &mut Vec<CascadeEntry<'a>>,
) {
    if rule.selector.pseudo_element != pseudo {
        return;
    }
    for decl in &rule.declarations {
        out.push(CascadeEntry {
            name: decl.name.as_str(),
            value: decl.value.as_str(),
            specificity: rule.specificity,
            important: decl.important,
            origin,
            layer_rank: rule.layer_rank(),
            important_layer_rank: rule.important_layer_rank(),
            order: rule.order,
        });
    }
}

pub fn compute_style(
    doc: &Document,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
    ua_index: &RuleIndex,
    author_index: &RuleIndex,
    ctx: &LayoutContext,
) -> ComputedStyle {
    compute_style_with_container(doc, node, parent_style, ua_index, author_index, ctx, None)
}

pub fn compute_style_with_container(
    doc: &Document,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
    ua_index: &RuleIndex,
    author_index: &RuleIndex,
    ctx: &LayoutContext,
    container: Option<ContainerQueryContext<'_>>,
) -> ComputedStyle {
    compute_style_full(
        doc,
        node,
        parent_style,
        ua_index,
        author_index,
        None,
        &[],
        ctx,
        container,
    )
}

struct MatchedRules<'a> {
    ua: Vec<std::rc::Rc<crate::rule_index::IndexedRule>>,
    host: Vec<std::rc::Rc<crate::rule_index::IndexedRule>>,
    slotted: &'a [std::rc::Rc<crate::rule_index::IndexedRule>],
    author: Vec<std::rc::Rc<crate::rule_index::IndexedRule>>,
}

impl MatchedRules<'_> {
    fn targets_pseudo(&self, pseudo: PseudoElement) -> bool {
        self.ua
            .iter()
            .chain(&self.host)
            .chain(self.slotted)
            .chain(&self.author)
            .any(|rule| rule.selector.pseudo_element == Some(pseudo))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn compute_style_full(
    doc: &Document,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
    ua_index: &RuleIndex,
    author_index: &RuleIndex,
    host_author_index: Option<&RuleIndex>,
    slotted_author_rules: &[std::rc::Rc<crate::rule_index::IndexedRule>],
    ctx: &LayoutContext,
    container: Option<ContainerQueryContext<'_>>,
) -> ComputedStyle {
    let (cw, ch, cname) = match container {
        Some(c) => (c.width, c.height, c.name),
        None => (ctx.viewport_width, ctx.viewport_height, None),
    };
    let matched = MatchedRules {
        ua: ua_index.match_element_with_container(
            doc,
            node,
            ctx.viewport_width,
            ctx.viewport_height,
            cw,
            ch,
            cname,
        ),
        host: host_author_index
            .map(|index| {
                index.match_element_with_container(
                    doc,
                    node,
                    ctx.viewport_width,
                    ctx.viewport_height,
                    cw,
                    ch,
                    cname,
                )
            })
            .unwrap_or_default(),
        slotted: slotted_author_rules,
        author: author_index.match_element_with_container(
            doc,
            node,
            ctx.viewport_width,
            ctx.viewport_height,
            cw,
            ch,
            cname,
        ),
    };

    let mut style =
        compute_style_from_matched(doc, node, parent_style, &matched, None, ctx, container);

    if matched.targets_pseudo(PseudoElement::Before) {
        let generated = compute_style_from_matched(
            doc,
            node,
            Some(&style),
            &matched,
            Some(PseudoElement::Before),
            ctx,
            container,
        );
        style.before = Some(Box::new(generated));
    }
    if matched.targets_pseudo(PseudoElement::After) {
        let generated = compute_style_from_matched(
            doc,
            node,
            Some(&style),
            &matched,
            Some(PseudoElement::After),
            ctx,
            container,
        );
        style.after = Some(Box::new(generated));
    }
    if matched.targets_pseudo(PseudoElement::Placeholder) {
        let generated = compute_style_from_matched(
            doc,
            node,
            Some(&style),
            &matched,
            Some(PseudoElement::Placeholder),
            ctx,
            container,
        );
        style.placeholder = Some(Box::new(generated));
    }

    style
}

#[allow(clippy::too_many_arguments)]
fn compute_style_from_matched(
    doc: &Document,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
    matched: &MatchedRules<'_>,
    pseudo: Option<PseudoElement>,
    ctx: &LayoutContext,
    _container: Option<ContainerQueryContext<'_>>,
) -> ComputedStyle {
    let mut style = if let Some(parent) = parent_style {
        ComputedStyle {
            display: Display::Inline,
            color: parent.color,
            background_color: CssColor::TRANSPARENT,
            font_size: parent.font_size,
            font_family: parent.font_family.clone(),
            font_weight: parent.font_weight,
            font_style: parent.font_style.clone(),
            line_height: parent.line_height,
            line_height_factor: parent.line_height_factor,
            line_height_normal: parent.line_height_normal,
            visibility: parent.visibility.clone(),
            pointer_events: parent.pointer_events.clone(),
            text_align: parent.text_align.clone(),
            white_space: parent.white_space.clone(),
            cursor: parent.cursor.clone(),
            variables: parent.variables.clone(),
            ..ComputedStyle::default()
        }
    } else {
        ComputedStyle::default()
    };

    let inline_declarations = if pseudo.is_none() {
        match doc.get(node) {
            Some(NodeData::Element(element)) => element
                .get_attribute("style")
                .map(|value| crate::stylesheet::parse_inline_style_shared(value))
                .unwrap_or_else(|| std::sync::Arc::new(Vec::new())),
            _ => std::sync::Arc::new(Vec::new()),
        }
    } else {
        std::sync::Arc::new(Vec::new())
    };

    let mut declarations: Vec<CascadeEntry<'_>> = Vec::new();

    for rule in &matched.ua {
        push_rule(rule, pseudo, Origin::UserAgent, &mut declarations);
    }

    for rule in &matched.host {
        let is_host_rule = rule.selector.chain.len() == 1
            && rule.selector.chain[0].predicates.iter().any(|p| {
                matches!(
                    p,
                    rix_dom::query::SimpleSelector::Pseudo(rix_dom::query::PseudoClass::Host(_))
                )
            });
        if is_host_rule {
            push_rule(rule, pseudo, Origin::ShadowHostAuthor, &mut declarations);
        }
    }

    for rule in matched.slotted {
        push_rule(rule, pseudo, Origin::ShadowHostAuthor, &mut declarations);
    }

    for rule in &matched.author {
        push_rule(rule, pseudo, Origin::Author, &mut declarations);
    }

    for decl in inline_declarations.iter() {
        declarations.push(CascadeEntry {
            name: decl.name.as_str(),
            value: decl.value.as_str(),
            specificity: Specificity::INLINE,
            important: decl.important,
            origin: Origin::Inline,
            layer_rank: usize::MAX,
            important_layer_rank: 0,
            order: usize::MAX,
        });
    }

    declarations.sort_by(|a, b| {
        if a.important != b.important {
            return a.important.cmp(&b.important);
        }
        let origin_key = |e: &CascadeEntry| -> u8 {
            match (e.important, e.origin) {
                (false, Origin::UserAgent) => 0,
                (false, Origin::ShadowHostAuthor) => 1,
                (false, _) => 2,
                (true, Origin::Author) => 0,
                (true, Origin::Inline) => 0,
                (true, Origin::ShadowHostAuthor) => 1,
                (true, Origin::UserAgent) => 2,
            }
        };
        let ao = origin_key(a);
        let bo = origin_key(b);
        if ao != bo {
            return ao.cmp(&bo);
        }
        let (al, bl) = if a.important {
            (a.important_layer_rank, b.important_layer_rank)
        } else {
            (a.layer_rank, b.layer_rank)
        };
        if al != bl {
            return al.cmp(&bl);
        }
        if a.specificity != b.specificity {
            return a.specificity.cmp(&b.specificity);
        }
        a.order.cmp(&b.order)
    });

    for entry in &declarations {
        if entry.name.starts_with("--") {
            let val = entry.value.trim();
            if val.eq_ignore_ascii_case("inherit") {
                
            } else if val.eq_ignore_ascii_case("initial")
                || val.eq_ignore_ascii_case("unset")
                || val.eq_ignore_ascii_case("revert")
                || val.eq_ignore_ascii_case("revert-layer")
            {
                std::sync::Arc::make_mut(&mut style.variables).remove(entry.name);
            } else {
                std::sync::Arc::make_mut(&mut style.variables)
                    .insert(entry.name.to_string(), entry.value.to_string());
            }
        }
    }

    let inherited_font_size = parent_style.map_or(ctx.root_font_size, |parent| parent.font_size);
    for entry in &declarations {
        if matches!(
            entry.name,
            "font" | "font-size" | "font-family" | "font-weight" | "font-style"
        ) {
            let resolved_val = resolve_var(entry.value, &style.variables);
            if matches!(entry.name, "font" | "font-size") {
                style.font_size = inherited_font_size;
            }
            apply_cascade_declaration(&mut style, parent_style, entry.name, &resolved_val, ctx);
        }
    }
    if let Some(factor) = style.line_height_factor {
        style.line_height = style.font_size * factor;
    }

    let is_document_element =
        matches!(doc.get(node), Some(NodeData::Element(_))) && doc.parent(node) == Some(doc.root());
    let computed_ctx = LayoutContext {
        root_font_size: if is_document_element {
            style.font_size
        } else {
            ctx.root_font_size
        },
        ..*ctx
    };

    if let Some(NodeData::Element(elem)) = doc.get(node) {
        if matches!(
            elem.name.local.as_str(),
            "img" | "svg" | "video" | "canvas" | "iframe" | "object" | "embed" | "input"
        ) {
            for property in ["width", "height"] {
                if let Some(value) = elem.get_attribute(property) {
                    apply_declaration(&mut style, property, value, &computed_ctx);
                }
            }
        }
    }

    let mut display_is_contents = false;

    for entry in declarations {
        if matches!(
            entry.name,
            "font" | "font-size" | "font-family" | "font-weight" | "font-style"
        ) {
            continue;
        }
        let resolved_val = resolve_var(entry.value, &style.variables);
        if entry.name == "display" {
            display_is_contents = is_contents_value(&resolved_val);
        }
        apply_cascade_declaration(
            &mut style,
            parent_style,
            entry.name,
            &resolved_val,
            &computed_ctx,
        );
    }

    blockify_display(&mut style, parent_style, display_is_contents);

    let flex_or_grid_item = parent_style.is_some_and(|parent| {
        matches!(
            parent.display,
            Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid
        )
    });
    if !flex_or_grid_item
        && style.min_height.is_none()
        && style.min_height_percent.is_none()
    {
        style.min_height = Some(0.0);
    }

    style
}

fn blockify_display(
    style: &mut ComputedStyle,
    parent_style: Option<&ComputedStyle>,
    display_is_contents: bool,
) {
    if display_is_contents {
        return;
    }
    let out_of_flow = matches!(style.position.as_str(), "absolute" | "fixed");
    let flex_or_grid_item = parent_style.is_some_and(|parent| {
        matches!(
            parent.display,
            Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid
        )
    });
    if out_of_flow || flex_or_grid_item {
        style.display = style.display.blockify();
    }
}

fn is_contents_value(value: &str) -> bool {
    value
        .trim()
        .split_whitespace()
        .last()
        .is_some_and(|keyword| keyword.eq_ignore_ascii_case("contents"))
}

pub fn compute_tree_styles(
    doc: &Document,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
    ua_index: &RuleIndex,
    author_index: &RuleIndex,
    ctx: &LayoutContext,
    out: &mut HashMap<NodeId, ComputedStyle>,
) {
    compute_tree_styles_with_container(
        doc,
        node,
        parent_style,
        ua_index,
        author_index,
        ctx,
        None,
        out,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn compute_tree_styles_with_container(
    doc: &Document,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
    ua_index: &RuleIndex,
    author_index: &RuleIndex,
    ctx: &LayoutContext,
    container: Option<ContainerQueryContext<'_>>,
    out: &mut HashMap<NodeId, ComputedStyle>,
) {
    compute_tree_styles_with_scoped_author(
        doc,
        node,
        parent_style,
        ua_index,
        author_index,
        &HashMap::new(),
        ctx,
        container,
        out,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn compute_tree_styles_with_scoped_author(
    doc: &Document,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
    ua_index: &RuleIndex,
    author_index: &RuleIndex,
    shadow_author_indexes: &HashMap<NodeId, RuleIndex>,
    ctx: &LayoutContext,
    container: Option<ContainerQueryContext<'_>>,
    out: &mut HashMap<NodeId, ComputedStyle>,
) {
    let initial_scope_root = doc.shadow_root_containing(node);
    let parent_has_sr = doc
        .parent(node)
        .and_then(|p| match doc.get(p) {
            Some(NodeData::Element(e)) => Some(e.shadow_root.is_some()),
            _ => None,
        })
        .unwrap_or(false);
    compute_tree_styles_inner(
        doc,
        node,
        parent_style,
        ua_index,
        author_index,
        shadow_author_indexes,
        initial_scope_root,
        parent_has_sr,
        ctx,
        container,
        out,
    );
}

#[allow(clippy::too_many_arguments)]
fn compute_tree_styles_inner(
    doc: &Document,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
    ua_index: &RuleIndex,
    author_index: &RuleIndex,
    shadow_author_indexes: &HashMap<NodeId, RuleIndex>,
    current_scope_root: Option<NodeId>,
    parent_has_shadow_root: bool,
    ctx: &LayoutContext,
    container: Option<ContainerQueryContext<'_>>,
    out: &mut HashMap<NodeId, ComputedStyle>,
) {
    if let Some(NodeData::Text(_)) = doc.get(node) {
        let style = if let Some(parent) = parent_style {
            ComputedStyle {
                display: Display::Inline,
                color: parent.color,
                background_color: CssColor::TRANSPARENT,
                font_size: parent.font_size,
                font_family: parent.font_family.clone(),
                font_weight: parent.font_weight,
                font_style: parent.font_style.clone(),
                line_height: parent.line_height,
                line_height_factor: parent.line_height_factor,
                line_height_normal: parent.line_height_normal,
                visibility: parent.visibility.clone(),
                pointer_events: parent.pointer_events.clone(),
                text_align: parent.text_align.clone(),
                white_space: parent.white_space.clone(),
                cursor: parent.cursor.clone(),
                variables: parent.variables.clone(),
                ..ComputedStyle::default()
            }
        } else {
            ComputedStyle::default()
        };
        out.insert(node, style);
        return;
    }

    let (node_scope_root, element_shadow_root) = match doc.get(node) {
        Some(NodeData::ShadowRoot { .. }) => (Some(node), None),
        Some(NodeData::Element(element)) => (current_scope_root, element.shadow_root),
        _ => (current_scope_root, None),
    };

    let scope_author_index = node_scope_root
        .and_then(|root| shadow_author_indexes.get(&root))
        .unwrap_or(author_index);

    let host_author_index = element_shadow_root
        .and_then(|root| shadow_author_indexes.get(&root));

    let slotted_rules = if parent_has_shadow_root {
        if let Some(slot) = doc.assigned_slot_for(node) {
            if let Some(slot_root) = doc.shadow_root_containing(slot) {
                if let Some(shadow_index) = shadow_author_indexes.get(&slot_root) {
                    let (cw, ch, cname) = match container {
                        Some(c) => (c.width, c.height, c.name),
                        None => (ctx.viewport_width, ctx.viewport_height, None),
                    };
                    shadow_index.match_slotted_element_with_container(
                        doc,
                        node,
                        slot,
                        ctx.viewport_width,
                        ctx.viewport_height,
                        cw,
                        ch,
                        cname,
                    )
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let style = compute_style_full(
        doc,
        node,
        parent_style,
        ua_index,
        scope_author_index,
        host_author_index,
        &slotted_rules,
        ctx,
        container,
    );
    out.insert(node, style.clone());
    let child_ctx = if matches!(doc.get(node), Some(NodeData::Element(_)))
        && doc.parent(node) == Some(doc.root())
    {
        LayoutContext {
            root_font_size: style.font_size,
            ..*ctx
        }
    } else {
        *ctx
    };

    let has_sr = element_shadow_root.is_some();
    doc.for_each_composed_child(node, |child| {
        compute_tree_styles_inner(
            doc,
            child,
            Some(&style),
            ua_index,
            author_index,
            shadow_author_indexes,
            node_scope_root,
            has_sr,
            &child_ctx,
            container,
            out,
        );
    });
}

pub(super) fn resolve_var<'a>(
    val: &'a str,
    vars: &HashMap<String, String>,
) -> std::borrow::Cow<'a, str> {
    if !val.contains("var(") {
        return std::borrow::Cow::Borrowed(val);
    }
    std::borrow::Cow::Owned(resolve_var_inner(val, vars, &mut Vec::new(), 0))
}

pub(super) fn resolve_var_inner<'v>(
    val: &str,
    vars: &'v HashMap<String, String>,
    stack: &mut Vec<&'v str>,
    depth: usize,
) -> String {
    const MAX_DEPTH: usize = 32;
    if !val.contains("var(") || depth > MAX_DEPTH {
        return if depth > MAX_DEPTH && val.contains("var(") {
            strip_unresolved_vars(val)
        } else {
            val.to_string()
        };
    }

    let mut result = String::with_capacity(val.len());
    let mut cursor = 0;
    let bytes = val.as_bytes();

    while cursor < bytes.len() {
        if let Some(pos) = val[cursor..].find("var(") {
            let start = cursor + pos;
            result.push_str(&val[cursor..start]);
            let mut var_depth = 1;
            let mut i = start + 4;
            let mut close_idx = None;
            while i < bytes.len() {
                match bytes[i] {
                    b'(' => var_depth += 1,
                    b')' => {
                        var_depth -= 1;
                        if var_depth == 0 {
                            close_idx = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }

            if let Some(end) = close_idx {
                let inner = &val[start + 4..end];
                let (prop_name, fallback) = match split_var_inner(inner) {
                    Some((p, fb)) => (p.trim(), Some(fb.trim())),
                    None => (inner.trim(), None),
                };

                if stack.contains(&prop_name) {
                    if let Some(fb) = fallback {
                        result.push_str(&resolve_var_inner(fb, vars, stack, depth + 1));
                    }
                } else if let Some((var_key, v)) = vars.get_key_value(prop_name) {
                    stack.push(var_key.as_str());
                    result.push_str(&resolve_var_inner(v, vars, stack, depth + 1));
                    stack.pop();
                } else if let Some(fb) = fallback {
                    result.push_str(&resolve_var_inner(fb, vars, stack, depth + 1));
                }
                cursor = end + 1;
            } else {
                result.push_str(&val[start..]);
                break;
            }
        } else {
            result.push_str(&val[cursor..]);
            break;
        }
    }

    result
}

pub(super) fn split_var_inner<'a>(inner: &'a str) -> Option<(&'a str, &'a str)> {
    let mut depth = 0i32;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                return Some((&inner[..i], &inner[i + 1..]));
            }
            _ => {}
        }
    }
    None
}

pub(super) fn strip_unresolved_vars(val: &str) -> String {
    let mut out = String::with_capacity(val.len());
    let mut cursor = 0;
    let bytes = val.as_bytes();
    while cursor < bytes.len() {
        if let Some(pos) = val[cursor..].find("var(") {
            let start = cursor + pos;
            out.push_str(&val[cursor..start]);
            let mut var_depth = 1;
            let mut i = start + 4;
            while i < bytes.len() && var_depth > 0 {
                match bytes[i] {
                    b'(' => var_depth += 1,
                    b')' => var_depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            cursor = i;
        } else {
            out.push_str(&val[cursor..]);
            break;
        }
    }
    out
}

pub(super) fn is_inherited_property(name: &str) -> bool {
    matches!(
        name,
        "color"
            | "font"
            | "font-family"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "line-height"
            | "visibility"
            | "pointer-events"
            | "text-align"
            | "white-space"
            | "letter-spacing"
            | "word-spacing"
            | "cursor"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_var_cow_zero_allocation() {
        let mut vars = HashMap::new();
        vars.insert("--brand-color".to_string(), "red".to_string());
        vars.insert("--alias-color".to_string(), "var(--brand-color)".to_string());

        let plain = "16px";
        let res = resolve_var(plain, &vars);
        assert!(matches!(res, std::borrow::Cow::Borrowed(_)));
        assert_eq!(&*res, "16px");

        let with_var = "var(--alias-color)";
        let res2 = resolve_var(with_var, &vars);
        assert_eq!(&*res2, "red");

        let with_fallback = "var(--missing, 24px)";
        let res3 = resolve_var(with_fallback, &vars);
        assert_eq!(&*res3, "24px");
    }
}
