use super::*;

pub(super) fn document_root(this: &JsValue, context: &mut Context) -> JsResult<NodeId> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let is_document = state
        .borrow()
        .document
        .borrow()
        .get(node)
        .is_some_and(|data| matches!(data, NodeData::Document));
    if !is_document {
        return Err(JsNativeError::typ()
            .with_message("Document method requires a Document receiver")
            .into());
    }
    Ok(node)
}

pub(super) fn document_get_revision(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let rev = doc.borrow().revision();
        Ok(JsValue::from(rev as f64))
    } else {
        Ok(JsValue::from(0.0))
    }
}

pub(super) fn document_get_title(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = document_root(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    let title = document.borrow().document_title(root);
    Ok(js_string!(title).into())
}

pub(super) fn document_set_title(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        let root = document_root(this, context)?;
        let value = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();
        let state = context
            .get_data::<Rc<RefCell<DomBridgeState>>>()
            .cloned()
            .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
        let document = state.borrow().document.clone();
        let (title, removed, added) = {
            let mut document = document.borrow_mut();
            let title = ensure_document_title_element(&mut document, root);
            let mut removed = Vec::new();
            let mut added = Vec::new();
            if let Some(title) = title {
                removed = document.children(title).collect();
                for child in &removed {
                    document.remove_child(*child);
                }
                if !value.is_empty() {
                    let text = document.create_text(&value);
                    document.append_child(title, text);
                    added.push(text);
                }
            }
            (title, removed, added)
        };
        for child in &added {
            state.borrow_mut().set_subtree_owner_document(*child, root);
        }
        if let Some(title) = title {
            record_child_mutation(context, title, &added, &removed, None, None)?;
            for child in &removed {
                super::custom_elements::notify_disconnected_subtree(*child, context);
            }
        }
        Ok(JsValue::undefined())
    })
}

fn ensure_document_title_element(document: &mut Document, root: NodeId) -> Option<NodeId> {
    if let Some(title) = document.document_title_element(root) {
        return Some(title);
    }
    let document_element = document
        .children(root)
        .find(|child| matches!(document.get(*child), Some(NodeData::Element(_))))?;
    let is_svg_root = matches!(
        document.get(document_element),
        Some(NodeData::Element(element))
            if &*element.name.ns == rix_dom::SVG_NAMESPACE && &*element.name.local == "svg"
    );
    if is_svg_root {
        let title = document.create_element_ns(rix_dom::SVG_NAMESPACE, None, "title");
        document.append_child(document_element, title);
        return Some(title);
    }
    let head = document.children(document_element).find(|child| {
        matches!(document.get(*child), Some(NodeData::Element(element))
            if &*element.name.ns == "http://www.w3.org/1999/xhtml"
                && &*element.name.local == "head")
    });
    let title = document.create_element("title");
    document.append_child(head.unwrap_or(document_element), title);
    Some(title)
}

pub(super) fn document_get_element_by_id(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let id_str = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let root = document_root(this, context)?;
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        let found = if root == doc_ref.root() {
            doc_ref.get_element_by_id(&id_str)
        } else {
            let selector = format!("#{}", id_str);
            doc_ref.query_selector(root, &selector)
        };
        if let Some(found_node) = found {
            drop(doc_ref);
            let wrapped = wrap_element(found_node, context);
            return Ok(wrapped.into());
        }
    }
    Ok(JsValue::null())
}

pub(super) fn document_query_selector(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let selector = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let root = document_root(this, context)?;
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        if let Some(found_node) = doc_ref.query_selector(root, &selector) {
            drop(doc_ref);
            let wrapped = wrap_element(found_node, context);
            return Ok(wrapped.into());
        }
    }
    Ok(JsValue::null())
}

pub(super) fn document_query_selector_all(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let selector = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let root = document_root(this, context)?;
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    let mut elements = Vec::new();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        let found = doc_ref.query_selector_all(root, &selector);
        drop(doc_ref);
        for node in found {
            elements.push(wrap_element(node, context).into());
        }
    }
    let array = boa_engine::object::builtins::JsArray::from_iter(elements, context);
    Ok(array.into())
}

pub(super) fn document_create_element(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let tag_name = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped()
        .to_lowercase();
    let root = document_root(this, context)?;
    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    if let Some(state) = state {
        let doc = state.borrow().document.clone();
        let node_id = {
            let mut document = doc.borrow_mut();
            let node = document.create_element(&tag_name);
            if tag_name == "template" {
                let fragment = document.new_node(NodeData::DocumentFragment);
                if let Some(NodeData::Element(element)) = document.get_mut(node) {
                    element.template_contents = Some(fragment);
                }
            }
            node
        };
        state.borrow_mut().set_subtree_owner_document(node_id, root);
        let wrapped = wrap_element(node_id, context);
        if tag_name.contains('-') {
            super::custom_elements::register_candidate(node_id, context)?;
        }
        return Ok(wrapped.into());
    }
    Ok(JsValue::null())
}

pub(super) fn document_active_element(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = document_root(this, context)?;
    let node = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|state| {
            let state = state.borrow();
            (root == state.main_document)
                .then(|| state.document.borrow().focused_node())
                .flatten()
        });
    Ok(node
        .map(|node| wrap_element(node, context).into())
        .unwrap_or(JsValue::null()))
}

fn document_is_visible(this: &JsValue, context: &mut Context) -> JsResult<bool> {
    let root = document_root(this, context)?;
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>() else {
        return Ok(false);
    };
    let state = state.borrow();
    Ok(root == state.main_document && state.page_visible)
}

pub(super) fn document_visibility_state(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let visibility = if document_is_visible(this, context)? {
        "visible"
    } else {
        "hidden"
    };
    Ok(js_string!(visibility).into())
}

pub(super) fn document_hidden(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::new(!document_is_visible(this, context)?))
}

pub(super) fn document_ready_state(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = document_root(this, context)?;
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>() else {
        return Ok(js_string!("loading").into());
    };
    let state = state.borrow();
    let ready_state = state
        .document_ready_states
        .get(&root)
        .map(|s| s.as_str())
        .unwrap_or_else(|| {
            if root == state.main_document {
                "loading"
            } else {
                "complete"
            }
        });
    Ok(js_string!(ready_state).into())
}

pub(super) fn document_current_script(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = document_root(this, context)?;
    let current = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|state| {
            let state = state.borrow();
            (root == state.main_document)
                .then_some(state.current_script)
                .flatten()
        });
    Ok(current
        .map(|node| wrap_element(node, context).into())
        .unwrap_or(JsValue::null()))
}

pub(super) fn document_base_uri(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = document_root(this, context)?;
    let value = super::hyperlink::document_base_url(root, context)
        .map(|url| url.to_string())
        .unwrap_or_else(|| "about:blank".to_string());
    Ok(js_string!(value).into())
}

pub(super) fn document_get_cookie(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let _root = document_root(this, context)?;
    let web = context.get_data::<Rc<RefCell<crate::web_api::WebApiState>>>().cloned();
    let Some(web) = web else {
        return Ok(js_string!("").into());
    };
    let (url, store) = {
        let borrowed = web.borrow();
        (borrowed.current_url.clone(), borrowed.cookie_store.clone())
    };
    let (Some(url), Some(store)) = (url, store) else {
        return Ok(js_string!("").into());
    };
    if url.scheme() != rix_core::url::RixScheme::Http && url.scheme() != rix_core::url::RixScheme::Https {
        return Ok(js_string!("").into());
    }
    let Ok(cookies) = store.get_cookies_for_url(&url) else {
        return Ok(js_string!("").into());
    };
    let cookie_str = cookies
        .into_iter()
        .filter(|c| !c.http_only)
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ");
    Ok(js_string!(cookie_str).into())
}

pub(super) fn document_set_cookie(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let _root = document_root(this, context)?;
    let raw = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();
    let web = context.get_data::<Rc<RefCell<crate::web_api::WebApiState>>>().cloned();
    let Some(web) = web else {
        return Ok(JsValue::undefined());
    };
    let (url, store) = {
        let borrowed = web.borrow();
        (borrowed.current_url.clone(), borrowed.cookie_store.clone())
    };
    let (Some(url), Some(store)) = (url, store) else {
        return Ok(JsValue::undefined());
    };
    if url.scheme() != rix_core::url::RixScheme::Http && url.scheme() != rix_core::url::RixScheme::Https {
        return Ok(JsValue::undefined());
    }
    let host = url.host_str().unwrap_or("");
    if let Some(cookie) = rix_net::parse_cookie_header(&raw, host) {
        if !cookie.http_only {
            let is_secure = url.scheme() == rix_core::url::RixScheme::Https;
            let _ = store.set_cookie_from_response(cookie.clone(), is_secure);
            web.borrow_mut().record_cookie_update(cookie);
        }
    }
    Ok(JsValue::undefined())
}

pub(super) fn document_get_default_view(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = document_root(this, context)?;
    let state_rc = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let state = state_rc.borrow();
    if state.inert_documents.contains(&root) {
        return Ok(JsValue::null());
    }
    if root == state.main_document {
        return Ok(context.global_object().into());
    }
    if let Some(iframe_node) = state.document_iframes.get(&root) {
        if super::iframe::is_connected_to_browsing_context(*iframe_node, &state) {
            if let Some(win) = state.iframe_windows.get(iframe_node) {
                return Ok(win.clone().into());
            }
        }
    }
    Ok(JsValue::null())
}

pub(super) fn document_has_default_view(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = document_root(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let has_default_view = !state.borrow().inert_documents.contains(&root);
    Ok(JsValue::new(has_default_view))
}

pub(super) fn document_create_html_document(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let title = if args.get_or_undefined(0).is_undefined() {
        String::new()
    } else {
        args.get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped()
    };
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    let root = {
        let mut document = document.borrow_mut();
        let root = document.new_node(NodeData::Document);
        let doctype = document.new_node(NodeData::DocumentType {
            name: "html".to_string(),
            public_id: String::new(),
            system_id: String::new(),
        });
        let html = document.create_element("html");
        let head = document.create_element("head");
        let title_element = document.create_element("title");
        let title_text = document.create_text(&title);
        let body = document.create_element("body");
        document.append_child(root, doctype);
        document.append_child(root, html);
        document.append_child(html, head);
        document.append_child(head, title_element);
        document.append_child(title_element, title_text);
        document.append_child(html, body);
        root
    };
    {
        let mut state = state.borrow_mut();
        state.inert_documents.insert(root);
        state.set_subtree_owner_document(root, root);
        state.document_ready_states.insert(root, "complete".to_string());
    }
    Ok(wrap_element(root, context).into())
}

pub(super) fn node_get_owner_document(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let owner = state.borrow().owner_document(node);
    Ok(owner
        .map(|owner| wrap_element(owner, context).into())
        .unwrap_or(JsValue::null()))
}

pub(super) fn document_doctype(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let document_node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    let doctype = {
        let document = document.borrow();
        if !matches!(document.get(document_node), Some(NodeData::Document)) {
            return Err(JsNativeError::typ()
                .with_message("Document.doctype requires a Document receiver")
                .into());
        }
        let mut doctype = None;
        for child in document.children(document_node) {
            if matches!(document.get(child), Some(NodeData::DocumentType { .. })) {
                doctype = Some(child);
                break;
            }
        }
        doctype
    };
    Ok(doctype
        .map(|node| wrap_element(node, context).into())
        .unwrap_or(JsValue::null()))
}

pub(super) fn document_create_comment(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let value = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let root = document_root(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    let node = document.borrow_mut().new_node(NodeData::Comment(value));
    state.borrow_mut().set_subtree_owner_document(node, root);
    Ok(wrap_element(node, context).into())
}

pub(super) fn template_get_content(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    let fragment = match document.borrow().get(node) {
        Some(NodeData::Element(element)) if &*element.name.local == "template" => {
            element.template_contents
        }
        _ => None,
    }
    .ok_or_else(|| JsNativeError::typ().with_message("Not an HTMLTemplateElement"))?;
    Ok(wrap_element(fragment, context).into())
}

pub(super) fn node_get_text_content(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let text = match document.get(node) {
        Some(NodeData::Document | NodeData::DocumentType { .. }) => return Ok(JsValue::null()),
        Some(NodeData::Text(value) | NodeData::Comment(value)) => value.clone(),
        _ => document.inner_text(node),
    };
    Ok(js_string!(text).into())
}

pub(super) fn node_set_text_content(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        let node = get_node_id_from_this(this, context)?;
        let value = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();
        let state = context
            .get_data::<Rc<RefCell<DomBridgeState>>>()
            .cloned()
            .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
        let owner = {
            let state_ref = state.borrow();
            state_ref
                .owner_document(node)
                .unwrap_or(state_ref.main_document)
        };
        let document = state.borrow().document.clone();
        let mut document = document.borrow_mut();
        let old_character = match document.get(node) {
            Some(NodeData::Text(text) | NodeData::Comment(text)) => Some(text.clone()),
            _ => None,
        };
        let mut removed = Vec::new();
        let mut added = Vec::new();
        match document.get_mut(node) {
            Some(NodeData::Text(text) | NodeData::Comment(text)) => *text = value,
            Some(NodeData::Element(_) | NodeData::DocumentFragment | NodeData::ShadowRoot { .. }) => {
                removed = document.children(node).collect();
                for child in &removed {
                    document.remove_child(*child);
                }
                if !value.is_empty() {
                    let text = document.create_text(&value);
                    document.append_child(node, text);
                    added.push(text);
                }
            }
            _ => {}
        }
        drop(document);
        for child in &added {
            state.borrow_mut().set_subtree_owner_document(*child, owner);
        }
        if let Some(old) = old_character {
            record_mutation(
                context,
                DomMutation {
                    target: node,
                    kind: "characterData",
                    attribute: None,
                    old_value: Some(&old),
                    added: &[],
                    removed: &[],
                    previous: None,
                    next: None,
                },
            )?;
        } else {
            record_child_mutation(context, node, &added, &removed, None, None)?;
            for child in &removed {
                super::custom_elements::notify_disconnected_subtree(*child, context);
            }
        }
        Ok(JsValue::undefined())
    })
}

pub(super) fn node_is_connected(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    let connected = super::shadow_dom::node_is_connected(node, &document.borrow());
    Ok(JsValue::new(connected))
}

pub(super) fn document_create_fragment(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = document_root(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let doc = state.borrow().document.clone();
    let node = doc.borrow_mut().new_node(NodeData::DocumentFragment);
    state.borrow_mut().set_subtree_owner_document(node, root);
    Ok(wrap_element(node, context).into())
}

pub(super) fn document_import_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        let target = document_root(this, context)?;
        let source = get_node_id_from_this(args.get_or_undefined(0), context)?;
        let deep = args.get_or_undefined(1).to_boolean();
        let state = context
            .get_data::<Rc<RefCell<DomBridgeState>>>()
            .cloned()
            .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
        let document = state.borrow().document.clone();
        if matches!(document.borrow().get(source), Some(NodeData::Document)) {
            return Err(JsNativeError::typ()
                .with_message("NotSupportedError: cannot import a Document")
                .into());
        }
        let clone = clone_subtree(&mut document.borrow_mut(), source, deep);
        state.borrow_mut().set_subtree_owner_document(clone, target);
        super::custom_elements::upgrade_subtree(clone, context)?;
        Ok(wrap_element(clone, context).into())
    })
}

pub(super) fn document_adopt_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        let target = document_root(this, context)?;
        let node = get_node_id_from_this(args.get_or_undefined(0), context)?;
        let state = context
            .get_data::<Rc<RefCell<DomBridgeState>>>()
            .cloned()
            .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
        let document = state.borrow().document.clone();
        if matches!(document.borrow().get(node), Some(NodeData::Document)) {
            return Err(JsNativeError::typ()
                .with_message("NotSupportedError: cannot adopt a Document")
                .into());
        }
        let old_document = state
            .borrow()
            .owner_document(node)
            .unwrap_or(state.borrow().main_document);
        let has_observers = state.borrow().active_mutation_observers > 0;
        let removals = if has_observers {
            Some(capture_removals(&document.borrow(), &[node]))
        } else {
            None
        };
        document.borrow_mut().remove_child(node);
        super::custom_elements::notify_disconnected_subtree(node, context);
        state.borrow_mut().set_subtree_owner_document(node, target);
        if let Some(removals) = removals {
            record_removals(context, removals)?;
        }
        if old_document != target {
            super::custom_elements::notify_adopted_subtree(node, old_document, target, context);
        }
        Ok(args.get_or_undefined(0).clone())
    })
}

pub(super) fn document_create_element_ns(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let namespace = if args.get_or_undefined(0).is_null() || args.get_or_undefined(0).is_undefined()
    {
        String::new()
    } else {
        args.get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped()
    };
    let name = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    let (prefix, local) = name
        .split_once(':')
        .map_or((None, name.as_str()), |(prefix, local)| {
            (Some(prefix), local)
        });
    if local.is_empty() || local.contains(':') || prefix == Some("") {
        return Err(JsNativeError::typ()
            .with_message("InvalidCharacterError: invalid qualified name")
            .into());
    }
    if prefix.is_some() && namespace.is_empty() {
        return Err(JsNativeError::typ()
            .with_message("NamespaceError: prefix requires a namespace")
            .into());
    }
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let root = document_root(this, context)?;
    let doc = state.borrow().document.clone();
    let node = doc
        .borrow_mut()
        .create_element_ns(&namespace, prefix, local);
    state.borrow_mut().set_subtree_owner_document(node, root);
    let wrapped = wrap_element(node, context);
    if (namespace.is_empty() || namespace == "http://www.w3.org/1999/xhtml") && local.contains('-') {
        super::custom_elements::register_candidate(node, context)?;
    }
    Ok(wrapped.into())
}
