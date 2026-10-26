
use super::*;
use url::Url;

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

pub(super) fn is_connected_to_browsing_context(
    node: NodeId,
    state: &DomBridgeState,
) -> bool {
    let document = state.document.borrow();
    let mut current = node;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        if matches!(document.get(current), Some(NodeData::Document)) {
            return !state.inert_documents.contains(&current);
        }
        if let Some(parent) = document.parent(current) {
            current = parent;
            continue;
        }
        match document.get(current) {
            Some(NodeData::ShadowRoot { host, .. }) => current = *host,
            _ => return false,
        }
    }
    false
}

pub(super) fn is_iframe_cross_origin_node(
    iframe_node: NodeId,
    context: &mut Context,
) -> bool {
    let state_rc = match context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        Some(s) => s,
        None => return false,
    };
    let srcdoc_or_src = {
        let state = state_rc.borrow();
        let doc = state.document.borrow();
        match doc.get(iframe_node) {
            Some(NodeData::Element(elem)) => {
                if elem.get_attribute("srcdoc").is_some() {
                    return false;
                }
                elem.get_attribute("src").map(str::to_owned)
            }
            _ => None,
        }
    };
    let Some(src) = srcdoc_or_src else {
        return false;
    };
    let trimmed = src.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("about:blank") {
        return false;
    }
    let base_url = hyperlink::document_base_url(iframe_node, context);
    let target_url = base_url
        .as_ref()
        .and_then(|base| base.join(trimmed).ok())
        .or_else(|| Url::parse(trimmed).ok());

    match (base_url, target_url) {
        (Some(base), Some(target)) => {
            if base.scheme() == "about" && target.scheme() != "about" {
                return true;
            }
            base.origin() != target.origin()
        }
        _ => false,
    }
}

fn js_is_iframe_cross_origin(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let el_val = args.get_or_undefined(0);
    let node = get_node_id_from_this(el_val, context)?;
    Ok(JsValue::new(is_iframe_cross_origin_node(node, context)))
}

pub(super) fn detach_iframe_context(iframe_node: NodeId, context: &mut Context) {
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        let mut state = state.borrow_mut();
        if let Some(doc_id) = state.iframe_documents.remove(&iframe_node) {
            state.document_iframes.remove(&doc_id);
            state.document_default_views.remove(&doc_id);
            state.document.borrow_mut().set_content_document(iframe_node, None);
        }
        state.iframe_windows.remove(&iframe_node);
    }
}

pub(super) fn notify_disconnected_iframe(node: NodeId, context: &mut Context) {
    let is_iframe = {
        let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
        if let Some(state) = state {
            let state = state.borrow();
            let document = state.document.borrow();
            matches!(document.get(node), Some(NodeData::Element(elem))
                if &*elem.name.ns == HTML_NAMESPACE && &*elem.name.local == "iframe")
        } else {
            false
        }
    };
    if is_iframe {
        detach_iframe_context(node, context);
    }
}

pub(super) fn prepare_connected_iframes(nodes: &[NodeId], context: &mut Context) -> JsResult<()> {
    let state_rc = match context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        Some(s) => s,
        None => return Ok(()),
    };
    let iframes: Vec<NodeId> = {
        let state = state_rc.borrow();
        let document = state.document.borrow();
        let mut result = Vec::new();
        for &node in nodes {
            let Some(NodeData::Element(elem)) = document.get(node) else {
                continue;
            };
            if document.children(node).next().is_none() {
                if &*elem.name.ns == HTML_NAMESPACE && &*elem.name.local == "iframe" {
                    result.push(node);
                }
                continue;
            }
            for candidate in document.descendants(node) {
                if matches!(document.get(candidate), Some(NodeData::Element(elem))
                    if &*elem.name.ns == HTML_NAMESPACE && &*elem.name.local == "iframe")
                {
                    result.push(candidate);
                }
            }
        }
        result
    };
    for iframe in iframes {
        let _ = ensure_iframe_context(iframe, context)?;
    }
    Ok(())
}

pub(super) fn ensure_iframe_context(
    iframe_node: NodeId,
    context: &mut Context,
) -> JsResult<(JsValue, JsValue)> {
    let state_rc = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;

    {
        let state = state_rc.borrow();
        if !is_connected_to_browsing_context(iframe_node, &state) {
            drop(state);
            detach_iframe_context(iframe_node, context);
            return Ok((JsValue::null(), JsValue::null()));
        }
    }

    {
        let state = state_rc.borrow();
        if let (Some(&doc_id), Some(win)) = (
            state.iframe_documents.get(&iframe_node),
            state.iframe_windows.get(&iframe_node),
        ) {
            let win_val = win.clone().into();
            drop(state);
            let doc_obj = wrap_element(doc_id, context);
            return Ok((win_val, doc_obj.into()));
        }
    }

    let srcdoc = {
        let state = state_rc.borrow();
        let doc = state.document.borrow();
        match doc.get(iframe_node) {
            Some(NodeData::Element(elem)) => elem.get_attribute("srcdoc").map(str::to_owned),
            _ => None,
        }
    };

    let doc_root = {
        let state = state_rc.borrow();
        let document = state.document.clone();
        let mut doc_mut = document.borrow_mut();
        let root = doc_mut.new_node(NodeData::Document);
        if let Some(srcdoc_content) = srcdoc {
            let parsed = rix_html::parse_html(&srcdoc_content);
            let parsed_root = parsed.root();
            for child in parsed.children(parsed_root) {
                let cloned = tree_ops::clone_subtree_into(&parsed, child, &mut doc_mut);
                doc_mut.append_child(root, cloned);
            }
        } else {
            let doctype = doc_mut.new_node(NodeData::DocumentType {
                name: "html".to_string(),
                public_id: String::new(),
                system_id: String::new(),
            });
            let html = doc_mut.create_element("html");
            let head = doc_mut.create_element("head");
            let body = doc_mut.create_element("body");
            doc_mut.append_child(root, doctype);
            doc_mut.append_child(root, html);
            doc_mut.append_child(html, head);
            doc_mut.append_child(html, body);
        }
        root
    };

    {
        let mut state = state_rc.borrow_mut();
        state.set_subtree_owner_document(doc_root, doc_root);
        state
            .document_ready_states
            .insert(doc_root, "complete".to_string());
        state
            .document
            .borrow_mut()
            .set_content_document(iframe_node, Some(doc_root));
    }

    let doc_obj = wrap_element(doc_root, context);
    let iframe_obj = wrap_element(iframe_node, context);
    let global_obj = context.global_object();

    let helper = global_obj
        .get(js_string!("__rixCreateIFrameWindow"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("__rixCreateIFrameWindow is unavailable"))?;

    let parent_win = {
        let bridge = state_rc.borrow();
        let doc = bridge.document.borrow();
        if let Some(container_doc_root) = super::script::node_document_root(iframe_node, &doc) {
            bridge
                .document_default_views
                .get(&container_doc_root)
                .cloned()
                .unwrap_or_else(|| global_obj.clone())
        } else {
            global_obj.clone()
        }
    };
    let top_win = global_obj
        .get(js_string!("top"), context)
        .unwrap_or_else(|_| global_obj.clone().into());

    let win_val = helper.call(
        &JsValue::undefined(),
        &[
            iframe_obj.into(),
            parent_win.into(),
            top_win,
            doc_obj.clone().into(),
        ],
        context,
    )?;

    let win_obj = win_val
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Failed to create iframe window"))?
        .clone();

    {
        let mut state = state_rc.borrow_mut();
        state.iframe_documents.insert(iframe_node, doc_root);
        state.document_iframes.insert(doc_root, iframe_node);
        state.iframe_windows.insert(iframe_node, win_obj.clone());
        state.document_default_views.insert(doc_root, win_obj);
    }

    Ok((win_val, doc_obj.into()))
}

pub(super) fn update_iframe_srcdoc(
    iframe_node: NodeId,
    srcdoc: &str,
    context: &mut Context,
) -> JsResult<()> {
    let state_rc = match context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        Some(s) => s,
        None => return Ok(()),
    };
    let (is_connected, doc_id) = {
        let state = state_rc.borrow();
        let is_connected = is_connected_to_browsing_context(iframe_node, &state);
        let doc_id = state.iframe_documents.get(&iframe_node).copied();
        (is_connected, doc_id)
    };
    if !is_connected {
        return Ok(());
    }
    let Some(doc_root) = doc_id else {
        let _ = ensure_iframe_context(iframe_node, context)?;
        return Ok(());
    };
    {
        let state = state_rc.borrow();
        let document = state.document.clone();
        let mut doc_mut = document.borrow_mut();
        let existing: Vec<_> = doc_mut.children(doc_root).collect();
        for child in existing {
            doc_mut.remove_child(child);
        }
        let parsed = rix_html::parse_html(srcdoc);
        let parsed_root = parsed.root();
        for child in parsed.children(parsed_root) {
            let cloned = tree_ops::clone_subtree_into(&parsed, child, &mut doc_mut);
            doc_mut.append_child(doc_root, cloned);
        }
    }
    {
        let mut state = state_rc.borrow_mut();
        state.set_subtree_owner_document(doc_root, doc_root);
        state
            .document_ready_states
            .insert(doc_root, "complete".to_string());
        state
            .document
            .borrow_mut()
            .set_content_document(iframe_node, Some(doc_root));
    }
    Ok(())
}

fn iframe_node(this: &JsValue, context: &mut Context) -> JsResult<NodeId> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let is_iframe = {
        let state = state.borrow();
        let document = state.document.borrow();
        matches!(document.get(node), Some(NodeData::Element(element))
            if &*element.name.ns == HTML_NAMESPACE && &*element.name.local == "iframe")
    };
    if !is_iframe {
        return Err(JsNativeError::typ()
            .with_message("HTMLIFrameElement accessor called on an incompatible receiver")
            .into());
    }
    Ok(node)
}

fn iframe_content_document_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let state_rc = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let is_connected = {
        let state = state_rc.borrow();
        is_connected_to_browsing_context(node, &state)
    };
    if !is_connected {
        return Ok(JsValue::null());
    }
    if is_iframe_cross_origin_node(node, context) {
        return Ok(JsValue::null());
    }
    let (_, doc_val) = ensure_iframe_context(node, context)?;
    Ok(doc_val)
}

fn iframe_content_window_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let state_rc = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let is_connected = {
        let state = state_rc.borrow();
        is_connected_to_browsing_context(node, &state)
    };
    if !is_connected {
        return Ok(JsValue::null());
    }
    let (win_val, _) = ensure_iframe_context(node, context)?;
    Ok(win_val)
}

fn iframe_src_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let raw = element_attr(node, "src", context)?;
    let Some(raw) = raw else {
        return Ok(js_string!("").into());
    };
    let resolved = hyperlink::document_base_url(node, context)
        .and_then(|base| base.join(&raw).ok())
        .or_else(|| Url::parse(&raw).ok());
    Ok(js_string!(resolved.map(|url| url.to_string()).unwrap_or(raw)).into())
}

fn iframe_src_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    iframe_node(this, context)?;
    let val = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    element_set_attribute(
        this,
        &[js_string!("src").into(), js_string!(val).into()],
        context,
    )
}

fn iframe_srcdoc_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let raw = element_attr(node, "srcdoc", context)?;
    Ok(js_string!(raw.unwrap_or_default()).into())
}

fn iframe_srcdoc_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let val = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    element_set_attribute(
        this,
        &[js_string!("srcdoc").into(), js_string!(val.clone()).into()],
        context,
    )?;
    update_iframe_srcdoc(node, &val, context)?;
    Ok(JsValue::undefined())
}

fn iframe_name_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let raw = element_attr(node, "name", context)?;
    Ok(js_string!(raw.unwrap_or_default()).into())
}

fn iframe_name_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    iframe_node(this, context)?;
    let val = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    element_set_attribute(
        this,
        &[js_string!("name").into(), js_string!(val).into()],
        context,
    )
}

fn iframe_width_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let raw = element_attr(node, "width", context)?;
    Ok(js_string!(raw.unwrap_or_default()).into())
}

fn iframe_width_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    iframe_node(this, context)?;
    let val = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    element_set_attribute(
        this,
        &[js_string!("width").into(), js_string!(val).into()],
        context,
    )
}

fn iframe_height_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let raw = element_attr(node, "height", context)?;
    Ok(js_string!(raw.unwrap_or_default()).into())
}

fn iframe_height_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    iframe_node(this, context)?;
    let val = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    element_set_attribute(
        this,
        &[js_string!("height").into(), js_string!(val).into()],
        context,
    )
}

fn iframe_loading_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let raw = element_attr(node, "loading", context)?;
    Ok(js_string!(raw.unwrap_or_default()).into())
}

fn iframe_loading_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    iframe_node(this, context)?;
    let val = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    element_set_attribute(
        this,
        &[js_string!("loading").into(), js_string!(val).into()],
        context,
    )
}

fn iframe_referrer_policy_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let raw = element_attr(node, "referrerpolicy", context)?;
    Ok(js_string!(raw.unwrap_or_default()).into())
}

fn iframe_referrer_policy_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    iframe_node(this, context)?;
    let val = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    element_set_attribute(
        this,
        &[js_string!("referrerpolicy").into(), js_string!(val).into()],
        context,
    )
}

fn iframe_allow_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let raw = element_attr(node, "allow", context)?;
    Ok(js_string!(raw.unwrap_or_default()).into())
}

fn iframe_allow_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    iframe_node(this, context)?;
    let val = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    element_set_attribute(
        this,
        &[js_string!("allow").into(), js_string!(val).into()],
        context,
    )
}

fn iframe_allow_fullscreen_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = iframe_node(this, context)?;
    let has = element_has_attr(node, "allowfullscreen", context)?;
    Ok(JsValue::new(has))
}

fn iframe_allow_fullscreen_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    iframe_node(this, context)?;
    let val = args.get_or_undefined(0).to_boolean();
    if val {
        element_set_attribute(
            this,
            &[js_string!("allowfullscreen").into(), js_string!("").into()],
            context,
        )?;
    } else {
        element_remove_attribute(this, &[js_string!("allowfullscreen").into()], context)?;
    }
    Ok(JsValue::undefined())
}

fn iframe_get_svg_document(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    iframe_node(this, context)?;
    Ok(JsValue::null())
}

fn element_attr(node: NodeId, name: &str, context: &mut Context) -> JsResult<Option<String>> {
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let state = state.borrow();
    let document = state.document.borrow();
    Ok(match document.get(node) {
        Some(NodeData::Element(element)) => element.get_attribute(name).map(str::to_owned),
        _ => None,
    })
}

fn element_has_attr(node: NodeId, name: &str, context: &mut Context) -> JsResult<bool> {
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let state = state.borrow();
    let document = state.document.borrow();
    Ok(match document.get(node) {
        Some(NodeData::Element(element)) => element.get_attribute(name).is_some(),
        _ => false,
    })
}

fn define_accessor(
    prototype: &JsObject,
    name: &str,
    getter: Option<NativeFunction>,
    setter: Option<NativeFunction>,
    context: &mut Context,
) -> JsResult<()> {
    let mut builder = PropertyDescriptor::builder()
        .enumerable(true)
        .configurable(true);
    if let Some(get_fn) = getter {
        let func = FunctionObjectBuilder::new(context.realm(), get_fn)
            .name(js_string!(format!("get {name}")))
            .build();
        builder = builder.get(func);
    }
    if let Some(set_fn) = setter {
        let func = FunctionObjectBuilder::new(context.realm(), set_fn)
            .name(js_string!(format!("set {name}")))
            .length(1)
            .build();
        builder = builder.set(func);
    }
    prototype.define_property_or_throw(js_string!(name), builder.build(), context)?;
    Ok(())
}

pub(super) fn register_iframe_interfaces(context: &mut Context) -> JsResult<()> {
    context.register_global_callable(
        js_string!("__rixIsIFrameCrossOrigin"),
        1,
        NativeFunction::from_fn_ptr(js_is_iframe_cross_origin),
    )?;

    let factory_script = r#"
        (function() {
            globalThis.__rixCreateIFrameWindow = function(frameElement, parentWindow, topWindow, docObj) {
                const win = Reflect.construct(EventTarget, [], Window);
                win.window = win;
                win.self = win;
                win.frames = win;
                win.parent = parentWindow;
                win.top = topWindow;
                win.__frameElement = frameElement;
                win.__doc = docObj;

                const isCrossOrigin = () => {
                    try {
                        return globalThis.__rixIsIFrameCrossOrigin(frameElement);
                    } catch (_) {
                        return false;
                    }
                };

                Object.defineProperty(win, 'frameElement', {
                    get() {
                        if (isCrossOrigin()) return null;
                        return frameElement;
                    },
                    configurable: true,
                    enumerable: true,
                });

                Object.defineProperty(win, 'document', {
                    get() {
                        if (isCrossOrigin()) {
                            throw new DOMException('Blocked a frame with origin "' + (win.location ? win.location.origin : 'null') + '" from accessing a cross-origin frame.', 'SecurityError');
                        }
                        return win.__doc;
                    },
                    configurable: true,
                    enumerable: true,
                });

                Object.defineProperty(win, 'name', {
                    get() { return frameElement.getAttribute('name') || ''; },
                    set(v) { frameElement.setAttribute('name', String(v)); },
                    configurable: true,
                    enumerable: true,
                });

                if (typeof globalThis.__rixRegisterWindow === 'function') {
                    globalThis.__rixRegisterWindow(win);
                }

                if (globalThis.setTimeout) {
                    win.setTimeout = function(callback, delay, ...args) {
                        if (typeof callback !== 'function') {
                            return globalThis.setTimeout.call(globalThis, callback, delay, ...args);
                        }
                        return globalThis.setTimeout.call(globalThis, function() {
                            if (typeof globalThis.__rixPushActiveWindow === 'function') {
                                globalThis.__rixPushActiveWindow(win);
                            }
                            try {
                                return callback.apply(win, args);
                            } finally {
                                if (typeof globalThis.__rixPopActiveWindow === 'function') {
                                    globalThis.__rixPopActiveWindow();
                                }
                            }
                        }, delay);
                    };
                }
                if (globalThis.clearTimeout) win.clearTimeout = globalThis.clearTimeout.bind(globalThis);
                if (globalThis.setInterval) win.setInterval = globalThis.setInterval.bind(globalThis);
                if (globalThis.clearInterval) win.clearInterval = globalThis.clearInterval.bind(globalThis);
                if (globalThis.requestAnimationFrame) win.requestAnimationFrame = globalThis.requestAnimationFrame.bind(globalThis);
                if (globalThis.cancelAnimationFrame) win.cancelAnimationFrame = globalThis.cancelAnimationFrame.bind(globalThis);
                if (globalThis.btoa) win.btoa = globalThis.btoa.bind(globalThis);
                if (globalThis.atob) win.atob = globalThis.atob.bind(globalThis);

                win.eval = function(code) {
                    if (typeof globalThis.__rixPushActiveWindow === 'function') {
                        globalThis.__rixPushActiveWindow(win);
                    }
                    try {
                        const fn = new Function('window', 'self', 'parent', 'top', 'document', 'location', 'return (' + code + ');');
                        return fn.call(win, win, win, win.parent, win.top, win.document, win.location);
                    } finally {
                        if (typeof globalThis.__rixPopActiveWindow === 'function') {
                            globalThis.__rixPopActiveWindow();
                        }
                    }
                };

                const location = {};
                const parseUrl = (str) => {
                    try { return new URL(str, globalThis.location ? globalThis.location.href : 'about:blank'); }
                    catch (_) { return null; }
                };
                const getLocationInfo = () => {
                    const srcdoc = frameElement.getAttribute('srcdoc');
                    if (srcdoc !== null) {
                        return {
                            href: 'about:srcdoc',
                            origin: globalThis.location ? globalThis.location.origin : 'null',
                            protocol: 'about:',
                            host: '',
                            hostname: '',
                            port: '',
                            pathname: 'srcdoc',
                            search: '',
                            hash: '',
                        };
                    }
                    const src = frameElement.getAttribute('src');
                    if (!src || src === 'about:blank') {
                        return {
                            href: 'about:blank',
                            origin: globalThis.location ? globalThis.location.origin : 'null',
                            protocol: 'about:',
                            host: '',
                            hostname: '',
                            port: '',
                            pathname: 'blank',
                            search: '',
                            hash: '',
                        };
                    }
                    const parsed = parseUrl(src);
                    if (!parsed) {
                        return {
                            href: src,
                            origin: 'null',
                            protocol: '',
                            host: '',
                            hostname: '',
                            port: '',
                            pathname: '',
                            search: '',
                            hash: '',
                        };
                    }
                    return {
                        href: parsed.href,
                        origin: parsed.origin,
                        protocol: parsed.protocol,
                        host: parsed.host,
                        hostname: parsed.hostname,
                        port: parsed.port,
                        pathname: parsed.pathname,
                        search: parsed.search,
                        hash: parsed.hash,
                    };
                };

                win.__rixGetOrigin = () => getLocationInfo().origin;

                const defineLocProp = (prop) => {
                    Object.defineProperty(location, prop, {
                        get() {
                            if (isCrossOrigin() && prop !== 'replace' && prop !== 'assign') {
                                throw new DOMException('Blocked a frame with origin "' + getLocationInfo().origin + '" from accessing a cross-origin frame.', 'SecurityError');
                            }
                            return getLocationInfo()[prop];
                        },
                        set(v) {
                            if (prop === 'href') {
                                frameElement.setAttribute('src', String(v));
                            }
                        },
                        configurable: true,
                        enumerable: true,
                    });
                };
                ['href', 'origin', 'protocol', 'host', 'hostname', 'port', 'pathname', 'search', 'hash'].forEach(defineLocProp);
                location.assign = function(url) { frameElement.setAttribute('src', String(url)); };
                location.replace = function(url) { frameElement.setAttribute('src', String(url)); };
                location.reload = function() {
                    const srcdoc = frameElement.getAttribute('srcdoc');
                    if (srcdoc !== null) frameElement.setAttribute('srcdoc', srcdoc);
                    else {
                        const src = frameElement.getAttribute('src') || 'about:blank';
                        frameElement.setAttribute('src', src);
                    }
                };
                location.toString = function() {
                    if (isCrossOrigin()) throw new DOMException('SecurityError', 'SecurityError');
                    return getLocationInfo().href;
                };
                location.valueOf = function() { return location.toString(); };
                win.location = location;

                return win;
            };
        })();
    "#;
    context.eval(boa_engine::Source::from_bytes(factory_script.as_bytes()))?;

    let constructor = context
        .global_object()
        .get(js_string!("HTMLIFrameElement"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("HTMLIFrameElement is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("HTMLIFrameElement prototype is unavailable")
        })?;

    define_accessor(
        &prototype,
        "contentDocument",
        Some(NativeFunction::from_fn_ptr(iframe_content_document_get)),
        None,
        context,
    )?;
    define_accessor(
        &prototype,
        "contentWindow",
        Some(NativeFunction::from_fn_ptr(iframe_content_window_get)),
        None,
        context,
    )?;
    define_accessor(
        &prototype,
        "src",
        Some(NativeFunction::from_fn_ptr(iframe_src_get)),
        Some(NativeFunction::from_fn_ptr(iframe_src_set)),
        context,
    )?;
    define_accessor(
        &prototype,
        "srcdoc",
        Some(NativeFunction::from_fn_ptr(iframe_srcdoc_get)),
        Some(NativeFunction::from_fn_ptr(iframe_srcdoc_set)),
        context,
    )?;
    define_accessor(
        &prototype,
        "name",
        Some(NativeFunction::from_fn_ptr(iframe_name_get)),
        Some(NativeFunction::from_fn_ptr(iframe_name_set)),
        context,
    )?;
    define_accessor(
        &prototype,
        "width",
        Some(NativeFunction::from_fn_ptr(iframe_width_get)),
        Some(NativeFunction::from_fn_ptr(iframe_width_set)),
        context,
    )?;
    define_accessor(
        &prototype,
        "height",
        Some(NativeFunction::from_fn_ptr(iframe_height_get)),
        Some(NativeFunction::from_fn_ptr(iframe_height_set)),
        context,
    )?;
    define_accessor(
        &prototype,
        "loading",
        Some(NativeFunction::from_fn_ptr(iframe_loading_get)),
        Some(NativeFunction::from_fn_ptr(iframe_loading_set)),
        context,
    )?;
    define_accessor(
        &prototype,
        "referrerPolicy",
        Some(NativeFunction::from_fn_ptr(iframe_referrer_policy_get)),
        Some(NativeFunction::from_fn_ptr(iframe_referrer_policy_set)),
        context,
    )?;
    define_accessor(
        &prototype,
        "allow",
        Some(NativeFunction::from_fn_ptr(iframe_allow_get)),
        Some(NativeFunction::from_fn_ptr(iframe_allow_set)),
        context,
    )?;
    define_accessor(
        &prototype,
        "allowFullscreen",
        Some(NativeFunction::from_fn_ptr(iframe_allow_fullscreen_get)),
        Some(NativeFunction::from_fn_ptr(iframe_allow_fullscreen_set)),
        context,
    )?;

    let svg_doc_fn = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(iframe_get_svg_document),
    )
    .name(js_string!("getSVGDocument"))
    .length(0)
    .build();
    prototype.define_property_or_throw(
        js_string!("getSVGDocument"),
        PropertyDescriptor::builder()
            .value(svg_doc_fn)
            .writable(true)
            .enumerable(true)
            .configurable(true)
            .build(),
        context,
    )?;

    let frame_elem_script = r#"
        (function(winProto) {
            Object.defineProperty(winProto, 'frameElement', {
                get: function() {
                    if (this === globalThis) return null;
                    if (this && this.__frameElement) return this.frameElement;
                    return null;
                },
                configurable: true,
                enumerable: true,
            });
        })(Window.prototype);
    "#;
    context.eval(boa_engine::Source::from_bytes(frame_elem_script.as_bytes()))?;

    Ok(())
}
