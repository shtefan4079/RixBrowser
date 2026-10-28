
use super::*;
use std::collections::HashSet;

#[derive(Clone)]
pub(super) struct CustomElementDefinition {
    pub(super) constructor: JsObject,
    pub(super) observed_attributes: Vec<String>,
    lifecycle_callbacks: Rc<HashMap<&'static str, JsObject>>,
}

#[derive(Clone)]
pub(crate) enum ElementReaction {
    Upgrade,
    Connected,
    Disconnected,
    Adopted {
        old_document: NodeId,
        new_document: NodeId,
    },
    AttributeChanged {
        name: String,
        old_value: Option<String>,
        new_value: Option<String>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UpgradeState {
    Upgrading,
    Custom,
    Failed,
}

#[derive(Clone)]
pub(super) struct CustomElementInstance {
    observed_attributes: HashSet<String>,
    lifecycle_callbacks: Rc<HashMap<&'static str, JsObject>>,
    state: UpgradeState,
    connected: bool,
}

pub(super) fn register_custom_elements(context: &mut Context) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!("CustomElementRegistry"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("CustomElementRegistry constructor missing")
        })?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("CustomElementRegistry prototype missing")
        })?;
    for (name, length, function) in [
        (
            "define",
            2,
            custom_elements_define as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
        ),
        ("get", 1, custom_elements_get),
        ("whenDefined", 1, custom_elements_when_defined),
        ("upgrade", 1, custom_elements_upgrade),
    ] {
        let method =
            FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(function))
                .name(js_string!(name))
                .length(length)
                .build();
        prototype.define_property_or_throw(
            js_string!(name),
            PropertyDescriptor::builder()
                .value(method)
                .writable(true)
                .configurable(true)
                .build(),
            context,
        )?;
    }

    let mut registry_builder = ObjectInitializer::new(context);
    let registry = registry_builder.build();
    registry.set_prototype(Some(prototype));
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    state.borrow_mut().custom_element_registry = Some(registry.clone());
    context.register_global_property(
        js_string!("customElements"),
        registry,
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;
    let root = state.borrow().main_document;
    upgrade_subtree(root, context)?;
    Ok(())
}

fn state_for_registry(
    this: &JsValue,
    context: &mut Context,
) -> JsResult<Rc<RefCell<DomBridgeState>>> {
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let registry = state.borrow().custom_element_registry.clone();
    if !registry
        .as_ref()
        .zip(this.as_object())
        .is_some_and(|(expected, actual)| JsObject::equals(expected, &actual))
    {
        return Err(JsNativeError::typ()
            .with_message("CustomElementRegistry method called on an incompatible receiver")
            .into());
    }
    Ok(state)
}

fn custom_elements_define(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = state_for_registry(this, context)?;
    if args.len() < 2 {
        return Err(JsNativeError::typ()
            .with_message("CustomElementRegistry.define requires a name and constructor")
            .into());
    }
    let name = args[0].to_string(context)?.to_std_string_escaped();
    let constructor = args[1]
        .as_object()
        .filter(JsObject::is_callable)
        .ok_or_else(|| {
            JsNativeError::typ().with_message("Custom element constructor must be callable")
        })?;
    let extension = match args.get(2).filter(|value| !value.is_null_or_undefined()) {
        None => None,
        Some(options) => {
            let options = options.as_object().ok_or_else(|| {
                JsNativeError::typ().with_message("Element definition options must be a dictionary")
            })?;
            let extension = options.get(js_string!("extends"), context)?;
            if extension.is_undefined() {
                None
            } else {
                Some(extension.to_string(context)?.to_std_string_escaped())
            }
        }
    };
    if !constructor.is_constructor() {
        return Err(JsNativeError::typ()
            .with_message("Custom element constructor is not constructable")
            .into());
    }
    if !is_valid_custom_element_name(&name) {
        return Err(dom_exception(
            "Invalid custom element name",
            "SyntaxError",
            context,
        )?);
    }
    if state
        .borrow()
        .custom_element_definitions
        .contains_key(&name)
        || state
            .borrow()
            .custom_element_definitions
            .values()
            .any(|definition| JsObject::equals(&definition.constructor, &constructor))
    {
        return Err(dom_exception(
            "The name or constructor is already registered",
            "NotSupportedError",
            context,
        )?);
    }
    if extension.is_some() {
        return Err(dom_exception(
            "Customized built-in elements are not supported by this DOM implementation",
            "NotSupportedError",
            context,
        )?);
    }

    if state.borrow().custom_element_definition_running {
        return Err(dom_exception(
            "A custom element definition is already running",
            "NotSupportedError",
            context,
        )?);
    }
    state.borrow_mut().custom_element_definition_running = true;
    let definition_result: JsResult<CustomElementDefinition> = (|| {
        let prototype = constructor
            .get(js_string!("prototype"), context)?
            .as_object()
            .ok_or_else(|| {
                JsNativeError::typ()
                    .with_message("Custom element constructor must have an object prototype")
            })?;
        let html_element = if let Some(proto) = state.borrow().prototypes.as_ref().map(|p| p.html_element.clone()) {
            proto
        } else {
            context
                .global_object()
                .get(js_string!("HTMLElement"), context)?
                .as_object()
                .ok_or_else(|| JsNativeError::error().with_message("HTMLElement constructor missing"))?
                .get(js_string!("prototype"), context)?
                .as_object()
                .ok_or_else(|| JsNativeError::error().with_message("HTMLElement prototype missing"))?
        };
        let mut parent = prototype.prototype();
        let mut extends_html_element = false;
        while let Some(current) = parent {
            if JsObject::equals(&current, &html_element) {
                extends_html_element = true;
                break;
            }
            parent = current.prototype();
        }
        if !extends_html_element {
            return Err(JsNativeError::typ()
                .with_message("Custom element constructor must extend HTMLElement")
                .into());
        }

        let mut lifecycle_callbacks = HashMap::new();
        for callback_name in [
            "connectedCallback",
            "disconnectedCallback",
            "connectedMoveCallback",
            "adoptedCallback",
            "attributeChangedCallback",
        ] {
            let callback = prototype.get(js_string!(callback_name), context)?;
            if callback.is_undefined() {
                continue;
            }
            let callback = callback
                .as_object()
                .filter(JsObject::is_callable)
                .ok_or_else(|| {
                    JsNativeError::typ().with_message(format!("{callback_name} must be callable"))
                })?;
            lifecycle_callbacks.insert(callback_name, callback);
        }
        let observed = if lifecycle_callbacks.contains_key("attributeChangedCallback") {
            constructor.get(js_string!("observedAttributes"), context)?
        } else {
            JsValue::undefined()
        };
        let observed_attributes = if observed.is_undefined() {
            Vec::new()
        } else {
            super::custom_element_sequence::convert_observed_attributes(&observed, context)?
        };

        Ok(CustomElementDefinition {
            constructor: constructor.clone(),
            observed_attributes,
            lifecycle_callbacks: Rc::new(lifecycle_callbacks),
        })
    })();
    state.borrow_mut().custom_element_definition_running = false;
    let definition = definition_result?;
    state
        .borrow_mut()
        .custom_element_definitions
        .insert(name.clone(), definition);
    with_ce_reactions(context, |context| {
        let candidates = state
            .borrow_mut()
            .custom_element_candidates
            .remove(&name)
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        for candidate in candidates {
            enqueue_reaction(&state, candidate, ElementReaction::Upgrade, context);
        }
        let waiters = state
            .borrow_mut()
            .custom_element_waiters
            .remove(&name)
            .unwrap_or_default();
        for resolve in waiters {
            if let Err(error) = resolve.call(
                &JsValue::undefined(),
                &[constructor.clone().into()],
                context,
            ) {
                crate::exceptions::report(error, context);
            }
        }
        let main_document = state.borrow().main_document;
        upgrade_matching_subtree(main_document, &name, context)?;
        Ok(JsValue::undefined())
    })
}

fn custom_elements_get(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = state_for_registry(this, context)?;
    let name = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let value = state
        .borrow()
        .custom_element_definitions
        .get(&name)
        .map(|definition| definition.constructor.clone().into())
        .unwrap_or(JsValue::undefined());
    Ok(value)
}

fn custom_elements_when_defined(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = state_for_registry(this, context)?;
    let name = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    if !is_valid_custom_element_name(&name) {
        return Ok(rejected_promise("Invalid custom element name", "SyntaxError", context)?.into());
    }
    if let Some(definition) = state.borrow().custom_element_definitions.get(&name) {
        let (promise, resolvers) = boa_engine::object::builtins::JsPromise::new_pending(context);
        resolvers.resolve.call(
            &JsValue::undefined(),
            &[definition.constructor.clone().into()],
            context,
        )?;
        return Ok(promise.into());
    }
    let (promise, resolvers) = boa_engine::object::builtins::JsPromise::new_pending(context);
    state
        .borrow_mut()
        .custom_element_waiters
        .entry(name)
        .or_default()
        .push(resolvers.resolve.into());
    Ok(promise.into())
}

fn custom_elements_upgrade(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    state_for_registry(this, context)?;
    let root = get_node_id_from_this(args.get_or_undefined(0), context)?;
    with_ce_reactions(context, |context| {
        upgrade_explicit_subtree(root, context)?;
        Ok(JsValue::undefined())
    })
}

pub(super) fn is_valid_custom_element_name(name: &str) -> bool {
    rix_dom::is_valid_custom_element_name(name)
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

fn rejected_promise(
    message: &str,
    name: &str,
    context: &mut Context,
) -> JsResult<boa_engine::object::builtins::JsPromise> {
    let (promise, resolvers) = boa_engine::object::builtins::JsPromise::new_pending(context);
    let exception = context
        .global_object()
        .get(js_string!("DOMException"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("DOMException constructor missing"))?
        .construct(
            &[js_string!(message).into(), js_string!(name).into()],
            None,
            context,
        )?;
    resolvers
        .reject
        .call(&JsValue::undefined(), &[exception.into()], context)?;
    Ok(promise)
}

fn ordered_subtree(root: NodeId, state: &Rc<RefCell<DomBridgeState>>) -> Vec<NodeId> {
    let document = state.borrow().document.clone();
    let document = document.borrow();
    if document.children(root).next().is_none() {
        if let Some(NodeData::Element(element)) = document.get(root) {
            if element.shadow_root.is_none() {
                return vec![root];
            }
        } else {
            return Vec::new();
        }
    }
    let mut ordered = Vec::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if matches!(document.get(node), Some(NodeData::Element(_))) {
            ordered.push(node);
        }
        pending.extend(document.children(node).rev());
        if let Some(NodeData::Element(element)) = document.get(node) {
            if let Some(shadow_root) = element.shadow_root {
                pending.push(shadow_root);
            }
        }
    }
    ordered
}

pub(super) fn upgrade_matching_subtree(
    root: NodeId,
    target_name: &str,
    context: &mut Context,
) -> JsResult<()> {
    if !target_name.contains('-') {
        return Ok(());
    }
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let candidates = {
        let bridge = state.borrow();
        let document = bridge.document.borrow();
        let mut result = Vec::new();
        let mut pending = Vec::with_capacity(128);
        pending.push(root);
        while let Some(node) = pending.pop() {
            if let Some(NodeData::Element(element)) = document.get(node) {
                if element.name.local.as_str() == target_name
                    && element.name.ns.as_str() == "http://www.w3.org/1999/xhtml"
                {
                    result.push(node);
                }
                if let Some(shadow_root) = element.shadow_root {
                    pending.push(shadow_root);
                }
            }
            for child in document.children(node) {
                pending.push(child);
            }
        }
        result
    };
    for node in candidates {
        register_candidate(node, context)?;
    }
    Ok(())
}

pub(super) fn upgrade_subtree(root: NodeId, context: &mut Context) -> JsResult<()> {
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let candidates = {
        let bridge = state.borrow();
        let document = bridge.document.borrow();
        let mut result = Vec::new();
        let mut pending = Vec::with_capacity(64);
        pending.push(root);
        while let Some(node) = pending.pop() {
            if let Some(NodeData::Element(element)) = document.get(node) {
                if element.name.local.contains('-') {
                    result.push(node);
                }
                if let Some(shadow_root) = element.shadow_root {
                    pending.push(shadow_root);
                }
            }
            for child in document.children(node) {
                pending.push(child);
            }
        }
        result
    };
    for node in candidates {
        register_candidate(node, context)?;
    }
    Ok(())
}

fn upgrade_explicit_subtree(root: NodeId, context: &mut Context) -> JsResult<()> {
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    for node in ordered_subtree(root, &state) {
        upgrade_element(node, context)?;
        if is_connected_to_main_document(node, &state) {
            notify_connected_node(node, context);
        }
    }
    Ok(())
}

pub(super) fn upgrade_element(node: NodeId, context: &mut Context) -> JsResult<()> {
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let (element_name, definition) = {
        let bridge = state.borrow();
        if bridge.custom_element_instances.contains_key(&node) {
            return Ok(());
        }
        let document = bridge.document.clone();
        let document = document.borrow();
        let Some(NodeData::Element(element)) = document.get(node) else {
            return Ok(());
        };
        if &*element.name.ns != "http://www.w3.org/1999/xhtml" || !element.name.local.contains('-') {
            return Ok(());
        }
        let Some(definition) = bridge.custom_element_definitions.get(element.name.local.as_str()) else {
            return Ok(());
        };
        (
            element.name.local.to_string(),
            definition.clone(),
        )
    };
    let candidates_empty = {
        let mut bridge = state.borrow_mut();
        bridge
            .custom_element_candidates
            .get_mut(&element_name)
            .is_some_and(|candidates| {
                candidates.remove(&node);
                candidates.is_empty()
            })
    };
    if candidates_empty {
        state
            .borrow_mut()
            .custom_element_candidates
            .remove(&element_name);
    }
    let wrapper = wrap_element(node, context);
    let prototype = definition
        .constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("Custom element constructor prototype is not an object")
        })?;
    if !wrapper.set_prototype(Some(prototype.clone())) {
        return Err(JsNativeError::typ()
            .with_message("Custom element prototype could not be installed")
            .into());
    }
    let instance = CustomElementInstance {
        observed_attributes: definition.observed_attributes.iter().cloned().collect(),
        lifecycle_callbacks: definition.lifecycle_callbacks.clone(),
        state: UpgradeState::Upgrading,
        connected: false,
    };
    state
        .borrow_mut()
        .custom_element_instances
        .insert(node, instance);

    let (push, pop) = {
        let bridge = state.borrow();
        (
            bridge.custom_element_push.clone().ok_or_else(|| {
                JsNativeError::error().with_message("Custom element construction bridge missing")
            })?,
            bridge.custom_element_pop.clone().ok_or_else(|| {
                JsNativeError::error().with_message("Custom element construction bridge missing")
            })?,
        )
    };
    push.call(
        &push.clone().into(),
        &[
            wrapper.clone().into(),
            definition.constructor.clone().into(),
        ],
        context,
    )?;
    let constructed = definition
        .constructor
        .construct(&[], Some(&definition.constructor), context);
    let _ = pop.call(&pop.clone().into(), &[], context)?;

    match constructed {
        Ok(instance) if JsObject::equals(&instance, &wrapper) => {
            if let Some(instance_state) = state.borrow_mut().custom_element_instances.get_mut(&node)
            {
                instance_state.state = UpgradeState::Custom;
            }
        }
        Ok(_) => {
            state
                .borrow_mut()
                .custom_element_instances
                .get_mut(&node)
                .map(|instance| instance.state = UpgradeState::Failed);
            crate::exceptions::report_message(
                "Custom element constructor returned a different object".to_string(),
                context,
            );
            return Ok(());
        }
        Err(error) => {
            state
                .borrow_mut()
                .custom_element_instances
                .get_mut(&node)
                .map(|instance| instance.state = UpgradeState::Failed);
            crate::exceptions::report(error, context);
            return Ok(());
        }
    }

    let attributes_to_notify = {
        let bridge = state.borrow();
        if let Some(instance) = bridge.custom_element_instances.get(&node) {
            if instance.observed_attributes.is_empty() {
                Vec::new()
            } else {
                let doc = bridge.document.borrow();
                match doc.get(node) {
                    Some(NodeData::Element(element)) => element
                        .attributes
                        .iter()
                        .filter(|(name, _)| instance.observed_attributes.contains(name.local.as_str()))
                        .map(|(name, value)| (name.local.to_string(), value.clone()))
                        .collect(),
                    _ => Vec::new(),
                }
            }
        } else {
            Vec::new()
        }
    };
    for (name, value) in attributes_to_notify {
        enqueue_reaction(
            &state,
            node,
            ElementReaction::AttributeChanged {
                name,
                old_value: None,
                new_value: Some(value),
            },
            context,
        );
    }
    if is_connected_to_main_document(node, &state) {
        notify_connected_node(node, context);
    }
    Ok(())
}

pub(super) fn register_candidate(node: NodeId, context: &mut Context) -> JsResult<()> {
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let (name, has_definition) = {
        let bridge = state.borrow();
        if bridge.custom_element_instances.contains_key(&node) {
            return Ok(());
        }
        if bridge.owner_document(node).unwrap_or(bridge.main_document) != bridge.main_document {
            return Ok(());
        }
        let document = bridge.document.clone();
        let document = document.borrow();
        match document.get(node) {
            Some(NodeData::Element(element))
                if &*element.name.ns == "http://www.w3.org/1999/xhtml" =>
            {
                if !element.name.local.contains('-') || !is_valid_custom_element_name(&element.name.local) {
                    return Ok(());
                }
                let has_def = bridge.custom_element_definitions.contains_key(&*element.name.local);
                if has_def {
                    (None, true)
                } else {
                    (Some(element.name.local.to_string()), false)
                }
            }
            _ => return Ok(()),
        }
    };
    if has_definition {
        upgrade_element(node, context)?;
    } else if let Some(name) = name {
        state
            .borrow_mut()
            .custom_element_candidates
            .entry(name)
            .or_default()
            .insert(node);
    }
    Ok(())
}

fn is_connected_to_main_document(node: NodeId, state: &Rc<RefCell<DomBridgeState>>) -> bool {
    let bridge = state.borrow();
    let document = bridge.document.clone();
    let connected = super::shadow_dom::node_is_connected(node, &document.borrow());
    connected
}

pub(super) fn with_ce_reactions<T, F>(context: &mut Context, f: F) -> JsResult<T>
where
    F: FnOnce(&mut Context) -> JsResult<T>,
{
    let state = match context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        Some(s) => s,
        None => return f(context),
    };
    state.borrow_mut().custom_element_reaction_stack.push((Vec::new(), HashSet::new()));

    let result = f(context);

    let (queue, _) = state
        .borrow_mut()
        .custom_element_reaction_stack
        .pop()
        .unwrap_or_default();

    invoke_reactions_in_queue(&queue, context);

    result
}

pub(super) fn invoke_reactions_in_queue(queue: &[NodeId], context: &mut Context) {
    let state = match context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        Some(s) => s,
        None => return,
    };
    for &node in queue {
        loop {
            let next_reaction = state
                .borrow_mut()
                .custom_element_reaction_queues
                .get_mut(&node)
                .and_then(|reactions| reactions.pop_front());
            let Some(reaction) = next_reaction else {
                break;
            };
            dispatch_reaction(node, reaction, context);
        }
    }
}

fn dispatch_reaction(node: NodeId, reaction: ElementReaction, context: &mut Context) {
    match reaction {
        ElementReaction::Upgrade => {
            let _ = upgrade_element(node, context);
        }
        ElementReaction::Connected => {
            invoke_lifecycle(node, "connectedCallback", &[], context);
        }
        ElementReaction::Disconnected => {
            invoke_lifecycle(node, "disconnectedCallback", &[], context);
        }
        ElementReaction::Adopted {
            old_document,
            new_document,
        } => {
            let old_doc = wrap_element(old_document, context);
            let new_doc = wrap_element(new_document, context);
            invoke_lifecycle(
                node,
                "adoptedCallback",
                &[old_doc.into(), new_doc.into()],
                context,
            );
        }
        ElementReaction::AttributeChanged {
            name,
            old_value,
            new_value,
        } => {
            invoke_lifecycle(
                node,
                "attributeChangedCallback",
                &[
                    js_string!(name).into(),
                    old_value.map_or(JsValue::null(), |v| js_string!(v).into()),
                    new_value.map_or(JsValue::null(), |v| js_string!(v).into()),
                ],
                context,
            );
        }
    }
}

fn enqueue_reaction(
    state: &Rc<RefCell<DomBridgeState>>,
    node: NodeId,
    reaction: ElementReaction,
    context: &mut Context,
) {
    let should_drain_backup = {
        let mut bridge = state.borrow_mut();
        bridge
            .custom_element_reaction_queues
            .entry(node)
            .or_default()
            .push_back(reaction);

        if let Some((current_queue, current_set)) = bridge.custom_element_reaction_stack.last_mut() {
            if current_set.insert(node) {
                current_queue.push(node);
            }
            false
        } else {
            if bridge.custom_element_backup_set.insert(node) {
                bridge.custom_element_backup_queue.push_back(node);
            }
            if bridge.custom_element_processing_backup {
                false
            } else {
                bridge.custom_element_processing_backup = true;
                true
            }
        }
    };

    if should_drain_backup {
        loop {
            let next_node = {
                let mut bridge = state.borrow_mut();
                if let Some(node) = bridge.custom_element_backup_queue.pop_front() {
                    bridge.custom_element_backup_set.remove(&node);
                    Some(node)
                } else {
                    None
                }
            };
            let Some(node) = next_node else {
                break;
            };
            loop {
                let next_reaction = state
                    .borrow_mut()
                    .custom_element_reaction_queues
                    .get_mut(&node)
                    .and_then(|reactions| reactions.pop_front());
                let Some(reaction) = next_reaction else {
                    break;
                };
                dispatch_reaction(node, reaction, context);
            }
        }
        state.borrow_mut().custom_element_processing_backup = false;
    }
}

fn invoke_lifecycle(node: NodeId, name: &str, args: &[JsValue], context: &mut Context) {
    let callback = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|state| {
            state
                .borrow()
                .custom_element_instances
                .get(&node)
                .and_then(|instance| instance.lifecycle_callbacks.get(name).cloned())
        });
    let Some(callback) = callback else {
        return;
    };
    let wrapper = wrap_element(node, context);
    if let Err(error) = callback.call(&wrapper.clone().into(), args, context) {
        crate::exceptions::report(error, context);
    }
}

pub(super) fn notify_connected_node(node: NodeId, context: &mut Context) {
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return;
    };
    let should_notify = {
        let mut bridge = state.borrow_mut();
        let Some(instance) = bridge.custom_element_instances.get_mut(&node) else {
            return;
        };
        if instance.state != UpgradeState::Custom
            || instance.connected
            || !instance.lifecycle_callbacks.contains_key("connectedCallback")
        {
            if instance.state == UpgradeState::Custom {
                instance.connected = true;
            }
            false
        } else {
            instance.connected = true;
            true
        }
    };
    if should_notify {
        enqueue_reaction(&state, node, ElementReaction::Connected, context);
    }
}

fn notify_disconnected_node(node: NodeId, context: &mut Context) {
    super::iframe::notify_disconnected_iframe(node, context);
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return;
    };
    let should_notify = {
        let mut bridge = state.borrow_mut();
        let Some(instance) = bridge.custom_element_instances.get_mut(&node) else {
            return;
        };
        if instance.state != UpgradeState::Custom
            || !instance.connected
            || !instance.lifecycle_callbacks.contains_key("disconnectedCallback")
        {
            if instance.state == UpgradeState::Custom {
                instance.connected = false;
            }
            false
        } else {
            instance.connected = false;
            true
        }
    };
    if should_notify {
        enqueue_reaction(&state, node, ElementReaction::Disconnected, context);
    }
}

pub(super) fn notify_connected_subtree(root: NodeId, context: &mut Context) -> JsResult<()> {
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    {
        let bridge = state.borrow();
        let document = bridge.document.borrow();
        let Some(NodeData::Element(element)) = document.get(root) else {
            return Ok(());
        };
        if document.children(root).next().is_none() && element.shadow_root.is_none() {
            let is_custom_candidate = element.name.local.contains('-');
            let is_instance = bridge.custom_element_instances.contains_key(&root);
            drop(document);
            drop(bridge);
            if is_custom_candidate {
                register_candidate(root, context)?;
            }
            if (is_instance || is_custom_candidate) && is_connected_to_main_document(root, &state) {
                notify_connected_node(root, context);
            }
            return Ok(());
        }
    }
    let (items_to_process, is_connected) = {
        let bridge = state.borrow();
        let doc = bridge.document.borrow();
        let connected = doc.is_connected(root);
        let mut items = Vec::new();
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            if let Some(NodeData::Element(element)) = doc.get(node) {
                let is_custom_candidate = element.name.local.contains('-');
                let is_instance = bridge.custom_element_instances.contains_key(&node);
                if is_custom_candidate || is_instance {
                    items.push((node, is_custom_candidate, is_instance));
                }
                if let Some(shadow_root) = element.shadow_root {
                    pending.push(shadow_root);
                }
            }
            pending.extend(doc.children(node).rev());
        }
        (items, connected)
    };
    for (node, is_custom_candidate, is_instance) in items_to_process {
        if is_custom_candidate {
            register_candidate(node, context)?;
        }
        if is_connected && (is_instance || is_custom_candidate) {
            notify_connected_node(node, context);
        }
    }
    Ok(())
}

pub(super) fn notify_disconnected_subtree(root: NodeId, context: &mut Context) {
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return;
    };
    {
        let bridge = state.borrow();
        let document = bridge.document.borrow();
        let Some(NodeData::Element(element)) = document.get(root) else {
            return;
        };
        if document.children(root).next().is_none() && element.shadow_root.is_none() {
            let is_custom_candidate = element.name.local.contains('-');
            let is_instance = bridge.custom_element_instances.contains_key(&root);
            let is_iframe = &*element.name.local == "iframe";
            drop(document);
            drop(bridge);
            if is_instance || is_custom_candidate || is_iframe {
                notify_disconnected_node(root, context);
            }
            return;
        }
    }
    let items = {
        let bridge = state.borrow();
        let doc = bridge.document.borrow();
        let mut items = Vec::new();
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            if let Some(NodeData::Element(element)) = doc.get(node) {
                let is_custom = bridge.custom_element_instances.contains_key(&node);
                let is_custom_candidate = element.name.local.contains('-');
                let is_iframe = &*element.name.local == "iframe";
                if is_custom || is_custom_candidate || is_iframe {
                    items.push(node);
                }
                if let Some(shadow_root) = element.shadow_root {
                    pending.push(shadow_root);
                }
            }
            pending.extend(doc.children(node).rev());
        }
        items
    };
    for node in items {
        notify_disconnected_node(node, context);
    }
}

pub(super) fn notify_adopted_subtree(
    root: NodeId,
    old_document: NodeId,
    new_document: NodeId,
    context: &mut Context,
) {
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return;
    };
    for node in ordered_subtree(root, &state) {
        let is_custom = state
            .borrow()
            .custom_element_instances
            .get(&node)
            .is_some_and(|instance| instance.state == UpgradeState::Custom);
        if is_custom {
            enqueue_reaction(
                &state,
                node,
                ElementReaction::Adopted {
                    old_document,
                    new_document,
                },
                context,
            );
        }
    }
}

pub(super) fn notify_attribute_changed(
    node: NodeId,
    name: &str,
    old_value: Option<&str>,
    new_value: Option<&str>,
    context: &mut Context,
) {
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return;
    };
    let observed = state
        .borrow()
        .custom_element_instances
        .get(&node)
        .is_some_and(|instance| {
            instance.state == UpgradeState::Custom && instance.observed_attributes.contains(name)
        });
    if observed && old_value != new_value {
        enqueue_reaction(
            &state,
            node,
            ElementReaction::AttributeChanged {
                name: name.to_string(),
                old_value: old_value.map(str::to_string),
                new_value: new_value.map(str::to_string),
            },
            context,
        );
    }
}
