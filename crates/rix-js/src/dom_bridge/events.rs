use super::*;
use boa_engine::object::builtins::JsArray;

pub(super) fn event_type_bubbles(event_type: &str) -> bool {
    !matches!(
        event_type,
        "focus" | "blur" | "load" | "unload" | "focusin" | "focusout" | "mouseenter" | "mouseleave" | "readystatechange"
    )
}

pub fn dispatch_dom_event_to_js(
    target_node: NodeId,
    event_type: &str,
    extra_props: Option<&[(&str, JsValue)]>,
    context: &mut Context,
) -> JsResult<JsValue> {
    dispatch_dom_event_object(target_node, event_type, extra_props, None, context)
}

pub(super) fn dispatch_existing_event_to_js(
    target_node: NodeId,
    event: JsObject,
    context: &mut Context,
) -> JsResult<JsValue> {
    if event
        .get(js_string!("preventDefault"), context)?
        .as_callable()
        .is_none()
    {
        return Err(JsNativeError::typ()
            .with_message("dispatchEvent requires an Event object")
            .into());
    }
    let event_type = event
        .get(js_string!("type"), context)?
        .to_string(context)?
        .to_std_string_escaped();
    if event_type.is_empty() {
        return Err(JsNativeError::typ()
            .with_message("InvalidStateError: cannot dispatch an event with an empty type")
            .into());
    }
    dispatch_dom_event_object(target_node, &event_type, None, Some(event), context)
}

fn dispatch_dom_event_object(
    target_node: NodeId,
    event_type: &str,
    extra_props: Option<&[(&str, JsValue)]>,
    existing_event: Option<JsObject>,
    context: &mut Context,
) -> JsResult<JsValue> {
    let state_rc = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    let Some(state) = state_rc else {
        return Ok(JsValue::undefined());
    };

    let mut bubbles = existing_event
        .as_ref()
        .and_then(|event| event.get(js_string!("bubbles"), context).ok())
        .map_or_else(
            || event_type_bubbles(event_type),
            |value| value.to_boolean(),
        );
    let mut cancelable = existing_event
        .as_ref()
        .and_then(|event| event.get(js_string!("cancelable"), context).ok())
        .map_or(true, |value| value.to_boolean());
    if let Some(props) = extra_props {
        for (k, v) in props {
            if *k == "bubbles" {
                bubbles = v.to_boolean();
            }
            if *k == "cancelable" {
                cancelable = v.to_boolean();
            }
        }
    }

    let mut composed = if let Some(event) = &existing_event {
        event.get(js_string!("composed"), context)?.to_boolean()
    } else {
        event_type_composed(event_type)
    };
    if let Some(props) = extra_props {
        for (key, value) in props {
            if *key == "composed" {
                composed = value.to_boolean();
            }
        }
    }

    let (event_path, capture_path, reaches_document) = {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        let event_path = build_event_path(&doc_ref, target_node, composed);
        let reaches_document = event_path
            .last()
            .is_some_and(|node| *node == doc_ref.root());
        let capture_path: Vec<NodeId> = event_path.iter().skip(1).rev().copied().collect();
        (event_path, capture_path, reaches_document)
    };

    let target_wrapped = wrap_element(target_node, context);
    let event_obj = if let Some(event_obj) = existing_event {
        if event_obj
            .get(js_string!("__rixDispatching"), context)?
            .to_boolean()
        {
            return Err(JsNativeError::typ()
                .with_message("InvalidStateError: event is already being dispatched")
                .into());
        }
        event_obj.set(
            js_string!("__rixDispatching"),
            JsValue::new(true),
            false,
            context,
        )?;
        event_obj.set(js_string!("target"), target_wrapped.clone(), false, context)?;
        event_obj.set(
            js_string!("currentTarget"),
            target_wrapped.clone(),
            false,
            context,
        )?;
        event_obj.set(js_string!("eventPhase"), JsValue::new(0), false, context)?;
        event_obj.set(
            js_string!("__rixStopped"),
            JsValue::new(false),
            false,
            context,
        )?;
        event_obj.set(
            js_string!("__rixImmediateStopped"),
            JsValue::new(false),
            false,
            context,
        )?;
        event_obj
    } else {
        let mut ev_builder = ObjectInitializer::new(context);
        ev_builder.property(
            js_string!("type"),
            js_string!(event_type),
            Attribute::READONLY | Attribute::ENUMERABLE,
        );
        ev_builder.property(
            js_string!("target"),
            target_wrapped.clone(),
            Attribute::WRITABLE | Attribute::CONFIGURABLE | Attribute::ENUMERABLE,
        );
        ev_builder.property(
            js_string!("currentTarget"),
            target_wrapped,
            Attribute::WRITABLE | Attribute::ENUMERABLE,
        );
        ev_builder.property(
            js_string!("bubbles"),
            JsValue::new(bubbles),
            Attribute::READONLY | Attribute::ENUMERABLE,
        );
        ev_builder.property(
            js_string!("cancelable"),
            JsValue::new(cancelable),
            Attribute::READONLY | Attribute::ENUMERABLE,
        );
        ev_builder.property(
            js_string!("composed"),
            JsValue::new(composed),
            Attribute::READONLY | Attribute::ENUMERABLE,
        );
        ev_builder.property(
            js_string!("eventPhase"),
            JsValue::new(0),
            Attribute::WRITABLE | Attribute::ENUMERABLE,
        );
        ev_builder.property(
            js_string!("timeStamp"),
            JsValue::new(0),
            Attribute::READONLY | Attribute::ENUMERABLE,
        );
        ev_builder.property(
            js_string!("defaultPrevented"),
            JsValue::new(false),
            Attribute::WRITABLE | Attribute::ENUMERABLE,
        );
        ev_builder.property(
            js_string!("__rixStopped"),
            JsValue::new(false),
            Attribute::WRITABLE,
        );
        ev_builder.property(
            js_string!("__rixImmediateStopped"),
            JsValue::new(false),
            Attribute::WRITABLE,
        );

        if let Some(props) = extra_props {
            for (k, v) in props {
                if *k == "bubbles" || *k == "cancelable" || *k == "composed" {
                    continue;
                }
                ev_builder.property(
                    js_string!(*k),
                    v.clone(),
                    Attribute::WRITABLE | Attribute::ENUMERABLE,
                );
            }
        }

        ev_builder.function(
            NativeFunction::from_fn_ptr(event_prevent_default),
            js_string!("preventDefault"),
            0,
        );
        ev_builder.function(
            NativeFunction::from_fn_ptr(event_stop_propagation),
            js_string!("stopPropagation"),
            0,
        );
        ev_builder.function(
            NativeFunction::from_fn_ptr(event_stop_immediate_propagation),
            js_string!("stopImmediatePropagation"),
            0,
        );
        let event = ev_builder.build();
        crate::structured_clone::mark_platform_object(context, &event)?;
        event
    };

    fn event_flag(evt: &JsObject, name: &str, ctx: &mut Context) -> bool {
        evt.get(js_string!(name), ctx)
            .ok()
            .and_then(|v| v.as_boolean())
            .unwrap_or(false)
    }

    fn event_prevent_default(
        this: &JsValue,
        _args: &[JsValue],
        context: &mut Context,
    ) -> JsResult<JsValue> {
        if let Some(obj) = this.as_object() {
            let in_passive = obj
                .get(js_string!("__rixInPassiveListener"), context)
                .ok()
                .and_then(|v| v.as_boolean())
                .unwrap_or(false);
            if in_passive {
                return Ok(JsValue::undefined());
            }
            let cancelable = obj
                .get(js_string!("cancelable"), context)
                .ok()
                .and_then(|v| v.as_boolean())
                .unwrap_or(true);
            if cancelable {
                let _ = obj.set(
                    js_string!("defaultPrevented"),
                    JsValue::new(true),
                    false,
                    context,
                );
            }
        }
        Ok(JsValue::undefined())
    }

    fn event_stop_propagation(
        this: &JsValue,
        _args: &[JsValue],
        context: &mut Context,
    ) -> JsResult<JsValue> {
        if let Some(obj) = this.as_object() {
            let _ = obj.set(
                js_string!("__rixStopped"),
                JsValue::new(true),
                false,
                context,
            );
        }
        Ok(JsValue::undefined())
    }

    fn event_stop_immediate_propagation(
        this: &JsValue,
        _args: &[JsValue],
        context: &mut Context,
    ) -> JsResult<JsValue> {
        if let Some(obj) = this.as_object() {
            let _ = obj.set(
                js_string!("__rixStopped"),
                JsValue::new(true),
                false,
                context,
            );
            let _ = obj.set(
                js_string!("__rixImmediateStopped"),
                JsValue::new(true),
                false,
                context,
            );
        }
        Ok(JsValue::undefined())
    }
    let is_stopped =
        |evt: &JsObject, ctx: &mut Context| -> bool { event_flag(evt, "__rixStopped", ctx) };
    let is_immediate_stopped = |evt: &JsObject, ctx: &mut Context| -> bool {
        event_flag(evt, "__rixImmediateStopped", ctx)
    };

    fn dispatch_window_listeners(
        event: &JsObject,
        capture: bool,
        phase: u8,
        context: &mut Context,
    ) -> JsResult<()> {
        let Some(dispatcher) = context
            .get_data::<crate::web_api::EventDispatchHooks>()
            .map(|hooks| hooks.dispatch_window.clone())
        else {
            return Ok(());
        };
        let result = dispatcher.call(
            &JsValue::undefined(),
            &[
                event.clone().into(),
                JsValue::new(capture),
                JsValue::new(phase),
            ],
            context,
        )?;
        if let Some(errors) = result.as_object() {
            let array = boa_engine::object::builtins::JsArray::from_object(errors.clone())?;
            for index in 0..array.length(context)? {
                let error = array.get(index, context)?.to_string(context)?;
                crate::exceptions::report_message(error.to_std_string_escaped(), context);
            }
        }
        Ok(())
    }

    if reaches_document {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        update_event_state(
            &event_obj,
            target_node,
            None,
            &event_path,
            reaches_document,
            &doc_ref,
            context,
        )?;
        drop(doc_ref);
        dispatch_window_listeners(&event_obj, true, 1, context)?;
    }

    fn invoke_listeners(
        node: NodeId,
        is_capture: bool,
        phase: u8,
        event_type: &str,
        ctx: &mut Context,
        state: &Rc<RefCell<DomBridgeState>>,
        evt: &JsObject,
    ) -> JsResult<()> {
        let listeners: Vec<JsListener> = state
            .borrow()
            .listeners
            .get(&(node, event_type.to_string()))
            .cloned()
            .unwrap_or_default();
        if listeners.is_empty() {
            return Ok(());
        }
        let this_wrapped = wrap_element(node, ctx);
        evt.set(js_string!("eventPhase"), JsValue::new(phase), false, ctx)?;
        let cur_val: JsValue = this_wrapped.clone().into();
        evt.set(js_string!("currentTarget"), cur_val, false, ctx)?;
        for listener in listeners {
            if event_flag(evt, "__rixImmediateStopped", ctx) {
                break;
            }
            if listener.capture != is_capture {
                continue;
            }
            if listener.once {
                state.borrow_mut().remove_listener(
                    node,
                    event_type,
                    &listener.callback,
                    Some(listener.capture),
                );
            }
            if listener.passive {
                let _ = evt.set(
                    js_string!("__rixInPassiveListener"),
                    JsValue::new(true),
                    false,
                    ctx,
                );
            }
            let res = if listener.callback.is_callable() {
                listener
                    .callback
                    .call(&this_wrapped.clone().into(), &[evt.clone().into()], ctx)
            } else if let Ok(handle_event) = listener.callback.get(js_string!("handleEvent"), ctx) {
                if let Some(func) = handle_event.as_callable() {
                    func.call(&listener.callback.clone().into(), &[evt.clone().into()], ctx)
                } else {
                    Ok(JsValue::undefined())
                }
            } else {
                Ok(JsValue::undefined())
            };
            if listener.passive {
                let _ = evt.set(
                    js_string!("__rixInPassiveListener"),
                    JsValue::new(false),
                    false,
                    ctx,
                );
            }
            if let Err(error) = res {
                crate::exceptions::report(error, ctx);
            }
        }
        Ok(())
    }

    for node in &capture_path {
        if is_stopped(&event_obj, context) {
            break;
        }
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        update_event_state(
            &event_obj,
            target_node,
            Some(*node),
            &event_path,
            reaches_document,
            &doc_ref,
            context,
        )?;
        let retargeted = retarget_event_target(&doc_ref, target_node, Some(*node));
        let phase = event_phase_for(*node, retargeted, true);
        drop(doc_ref);
        invoke_listeners(*node, true, phase, event_type, context, &state, &event_obj)?;
    }

    let reached_target = !is_stopped(&event_obj, context);
    if reached_target {
        let doc = state.borrow().document.clone();
        let doc_ref = doc.borrow();
        update_event_state(
            &event_obj,
            target_node,
            Some(target_node),
            &event_path,
            reaches_document,
            &doc_ref,
            context,
        )?;
        drop(doc_ref);
        invoke_listeners(
            target_node,
            true,
            2,
            event_type,
            context,
            &state,
            &event_obj,
        )?;
        if !is_immediate_stopped(&event_obj, context) {
            let was_stopped = is_stopped(&event_obj, context);
            if was_stopped {
                event_obj.set(
                    js_string!("__rixStopped"),
                    JsValue::new(false),
                    false,
                    context,
                )?;
            }
            invoke_listeners(
                target_node,
                false,
                2,
                event_type,
                context,
                &state,
                &event_obj,
            )?;
            if was_stopped {
                event_obj.set(
                    js_string!("__rixStopped"),
                    JsValue::new(true),
                    false,
                    context,
                )?;
            }
        }
    }

    if bubbles && !is_stopped(&event_obj, context) {
        for node in capture_path.iter().rev() {
            if is_stopped(&event_obj, context) {
                break;
            }
            let doc = state.borrow().document.clone();
            let doc_ref = doc.borrow();
            update_event_state(
                &event_obj,
                target_node,
                Some(*node),
                &event_path,
                reaches_document,
                &doc_ref,
                context,
            )?;
            let retargeted = retarget_event_target(&doc_ref, target_node, Some(*node));
            let phase = event_phase_for(*node, retargeted, false);
            drop(doc_ref);
            invoke_listeners(*node, false, phase, event_type, context, &state, &event_obj)?;
        }
        if reaches_document && !is_stopped(&event_obj, context) {
            let doc = state.borrow().document.clone();
            let doc_ref = doc.borrow();
            update_event_state(
                &event_obj,
                target_node,
                None,
                &event_path,
                reaches_document,
                &doc_ref,
                context,
            )?;
            drop(doc_ref);
            dispatch_window_listeners(&event_obj, false, 3, context)?;
        }
    }

    if let Some(hooks) = context
        .get_data::<crate::web_api::EventDispatchHooks>()
        .cloned()
    {
        hooks
            .clear_path
            .call(&JsValue::undefined(), &[event_obj.clone().into()], context)?;
    }

    let prevented = event_obj
        .get(js_string!("defaultPrevented"), context)?
        .to_boolean();
    event_obj.set(js_string!("currentTarget"), JsValue::null(), false, context)?;
    event_obj.set(js_string!("eventPhase"), JsValue::new(0), false, context)?;
    event_obj.set(
        js_string!("__rixDispatching"),
        JsValue::new(false),
        false,
        context,
    )?;
    Ok(JsValue::new(!prevented))
}

fn event_type_composed(event_type: &str) -> bool {
    matches!(
        event_type,
        "auxclick"
            | "beforeinput"
            | "blur"
            | "change"
            | "click"
            | "compositionend"
            | "compositionstart"
            | "compositionupdate"
            | "contextmenu"
            | "copy"
            | "cut"
            | "dblclick"
            | "drag"
            | "dragend"
            | "dragenter"
            | "dragleave"
            | "dragover"
            | "dragstart"
            | "drop"
            | "focus"
            | "focusin"
            | "focusout"
            | "keydown"
            | "keypress"
            | "keyup"
            | "mousedown"
            | "mouseenter"
            | "mouseleave"
            | "mousemove"
            | "mouseout"
            | "mouseover"
            | "mouseup"
            | "paste"
            | "pointercancel"
            | "pointerdown"
            | "pointerenter"
            | "pointerleave"
            | "pointermove"
            | "pointerout"
            | "pointerover"
            | "pointerrawupdate"
            | "pointerup"
            | "touchcancel"
            | "touchend"
            | "touchmove"
            | "touchstart"
            | "wheel"
    )
}

fn build_event_path(document: &Document, target: NodeId, composed: bool) -> Vec<NodeId> {
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let mut current = Some(target);
    while let Some(node) = current {
        if !seen.insert(node) {
            break;
        }
        path.push(node);
        current = document
            .assigned_slot_for(node)
            .or_else(|| document.parent(node))
            .or_else(|| {
                if composed {
                    match document.get(node) {
                        Some(NodeData::ShadowRoot { host, .. }) => Some(*host),
                        _ => None,
                    }
                } else {
                    None
                }
            });
    }
    path
}

fn shadow_tree_scopes(document: &Document, node: NodeId) -> Vec<NodeId> {
    let mut scopes = Vec::new();
    let mut current = node;
    let mut seen = HashSet::new();
    loop {
        let Some(root) = document.shadow_root_containing(current) else {
            break;
        };
        if !seen.insert(root) {
            break;
        }
        scopes.push(root);
        let Some(NodeData::ShadowRoot { host, .. }) = document.get(root) else {
            break;
        };
        current = *host;
    }
    scopes
}

fn retarget_event_target(
    document: &Document,
    target: NodeId,
    current_target: Option<NodeId>,
) -> NodeId {
    let current_scopes = current_target.map(|node| shadow_tree_scopes(document, node));
    let mut candidate = target;
    let mut seen = HashSet::new();
    loop {
        let Some(root) = document.shadow_root_containing(candidate) else {
            return candidate;
        };
        if !seen.insert(root) {
            return candidate;
        }
        if current_scopes
            .as_ref()
            .is_some_and(|scopes| scopes.contains(&root))
        {
            return candidate;
        }
        let Some(NodeData::ShadowRoot { host, .. }) = document.get(root) else {
            return candidate;
        };
        candidate = *host;
    }
}

fn visible_event_path(
    document: &Document,
    path: &[NodeId],
    current_target: Option<NodeId>,
) -> Vec<NodeId> {
    let current_scopes = current_target.map(|node| shadow_tree_scopes(document, node));
    path.iter()
        .copied()
        .filter(|node| {
            shadow_tree_scopes(document, *node).into_iter().all(|root| {
                let is_closed = matches!(
                    document.get(root),
                    Some(NodeData::ShadowRoot { mode, .. }) if mode == "closed"
                );
                !is_closed
                    || current_scopes
                        .as_ref()
                        .is_some_and(|scopes| scopes.contains(&root))
            })
        })
        .collect()
}

fn event_phase_for(current_target: NodeId, retargeted_target: NodeId, capture: bool) -> u8 {
    if current_target == retargeted_target {
        2
    } else if capture {
        1
    } else {
        3
    }
}

fn update_event_state(
    event: &JsObject,
    original_target: NodeId,
    current_target: Option<NodeId>,
    event_path: &[NodeId],
    reaches_document: bool,
    document: &Document,
    context: &mut Context,
) -> JsResult<()> {
    let target = retarget_event_target(document, original_target, current_target);
    let target_value: JsValue = wrap_element(target, context).into();
    let visible_path = visible_event_path(document, event_path, current_target);
    let mut path_values = visible_path
        .into_iter()
        .map(|node| JsValue::from(wrap_element(node, context)))
        .collect::<Vec<_>>();
    if reaches_document {
        path_values.push(context.global_object().into());
    }
    let path: JsValue = JsArray::from_iter(path_values, context).into();
    if let Some(hooks) = context
        .get_data::<crate::web_api::EventDispatchHooks>()
        .cloned()
    {
        hooks.set_path.call(
            &JsValue::undefined(),
            &[event.clone().into(), target_value, path],
            context,
        )?;
    } else {
        event.set(js_string!("target"), target_value, false, context)?;
    }
    Ok(())
}
