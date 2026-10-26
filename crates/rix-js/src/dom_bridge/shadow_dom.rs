use super::*;

pub(super) fn element_attach_shadow(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let host = get_node_id_from_this(this, context)?;
    let options = args.get_or_undefined(0);
    let options = if options.is_null() || options.is_undefined() {
        None
    } else {
        Some(options.to_object(context)?)
    };
    let mode = match &options {
        Some(options) => options.get(js_string!("mode"), context)?,
        None => JsValue::undefined(),
    };
    if mode.is_undefined() {
        return Err(JsNativeError::typ()
            .with_message("ShadowRootInit.mode is required")
            .into());
    }
    let mode = mode.to_string(context)?.to_std_string_escaped();
    if !matches!(mode.as_str(), "open" | "closed") {
        return Err(JsNativeError::typ()
            .with_message("ShadowRootInit.mode must be 'open' or 'closed'")
            .into());
    }
    let delegates_focus = match &options {
        Some(options) => options
            .get(js_string!("delegatesFocus"), context)?
            .to_boolean(),
        None => false,
    };
    let clonable = match &options {
        Some(options) => options
            .get(js_string!("clonable"), context)?
            .to_boolean(),
        None => false,
    };

    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    let root = {
        let mut document = document.borrow_mut();
        let Some(NodeData::Element(element)) = document.get(host) else {
            return Err(JsNativeError::typ()
                .with_message("Element.attachShadow called on an incompatible receiver")
                .into());
        };
        let local_name = element.name.local.as_ref();
        let html_namespace = &*element.name.ns == "http://www.w3.org/1999/xhtml";
        if !html_namespace || !rix_dom::node::is_valid_shadow_host_name(local_name) {
            return Err(dom_exception(
                "This element cannot host a shadow root",
                "NotSupportedError",
                context,
            )?);
        }
        if element.shadow_root.is_some() {
            return Err(dom_exception(
                "This element already has a shadow root",
                "NotSupportedError",
                context,
            )?);
        }
        let root = document.new_node(NodeData::ShadowRoot {
            host,
            mode: mode.clone(),
            delegates_focus,
            clonable,
        });
        if let Some(NodeData::Element(element)) = document.get_mut(host) {
            element.shadow_root = Some(root);
        }
        root
    };
    let owner = state
        .borrow()
        .owner_document(host)
        .unwrap_or(state.borrow().main_document);
    state.borrow_mut().set_subtree_owner_document(root, owner);
    Ok(wrap_element(root, context).into())
}

pub(super) fn element_get_shadow_root(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let host = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    let root = {
        let document = document.borrow();
        let Some(NodeData::Element(element)) = document.get(host) else {
            return Err(JsNativeError::typ()
                .with_message("Element.shadowRoot called on an incompatible receiver")
                .into());
        };
        element.shadow_root.filter(|root| {
            matches!(document.get(*root), Some(NodeData::ShadowRoot { mode, .. }) if mode == "open")
        })
    };
    Ok(root.map_or(JsValue::null(), |root| wrap_element(root, context).into()))
}

pub(super) fn shadow_root_get_host(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = get_node_id_from_this(this, context)?;
    let document = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?
        .borrow()
        .document
        .clone();
    let host = match document.borrow().get(root) {
        Some(NodeData::ShadowRoot { host, .. }) => *host,
        _ => {
            return Err(JsNativeError::typ()
                .with_message("ShadowRoot.host called on an incompatible receiver")
                .into());
        }
    };
    Ok(wrap_element(host, context).into())
}

pub(super) fn shadow_root_get_mode(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = get_node_id_from_this(this, context)?;
    let document = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?
        .borrow()
        .document
        .clone();
    let mode = match document.borrow().get(root) {
        Some(NodeData::ShadowRoot { mode, .. }) => Some(mode.clone()),
        _ => None,
    };
    match mode {
        Some(mode) => Ok(js_string!(mode).into()),
        None => Err(JsNativeError::typ()
            .with_message("ShadowRoot.mode called on an incompatible receiver")
            .into()),
    }
}

pub(super) fn shadow_root_get_delegates_focus(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = get_node_id_from_this(this, context)?;
    let document = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?
        .borrow()
        .document
        .clone();
    let delegates_focus = match document.borrow().get(root) {
        Some(NodeData::ShadowRoot {
            delegates_focus, ..
        }) => Some(*delegates_focus),
        _ => None,
    };
    match delegates_focus {
        Some(delegates_focus) => Ok(JsValue::new(delegates_focus)),
        None => Err(JsNativeError::typ()
            .with_message("ShadowRoot.delegatesFocus called on an incompatible receiver")
            .into()),
    }
}

pub(super) fn shadow_root_get_clonable(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let root = get_node_id_from_this(this, context)?;
    let document = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?
        .borrow()
        .document
        .clone();
    let clonable = match document.borrow().get(root) {
        Some(NodeData::ShadowRoot { clonable, .. }) => Some(*clonable),
        _ => None,
    };
    match clonable {
        Some(clonable) => Ok(JsValue::new(clonable)),
        None => Err(JsNativeError::typ()
            .with_message("ShadowRoot.clonable called on an incompatible receiver")
            .into()),
    }
}

pub(super) fn node_get_root_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let mut node = get_node_id_from_this(this, context)?;
    let composed = args.first().is_some_and(JsValue::to_boolean);
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    loop {
        if let Some(parent) = document.borrow().parent(node) {
            node = parent;
            continue;
        }
        if composed {
            let host = match document.borrow().get(node) {
                Some(NodeData::ShadowRoot { host, .. }) => Some(*host),
                _ => None,
            };
            if let Some(host) = host {
                node = host;
                continue;
            }
        }
        return Ok(wrap_element(node, context).into());
    }
}

pub(super) fn node_is_connected(node: NodeId, document: &Document) -> bool {
    document.is_connected(node)
}

fn dom_exception(message: &str, name: &str, context: &mut Context) -> JsResult<JsError> {
    let constructor = context
        .global_object()
        .get(js_string!("DOMException"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("DOMException constructor missing"))?;
    let exception = constructor.construct(
        &[js_string!(message).into(), js_string!(name).into()],
        None,
        context,
    )?;
    Ok(JsError::from_opaque(exception.into()))
}
