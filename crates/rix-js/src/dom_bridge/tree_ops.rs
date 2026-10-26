use super::*;

pub(super) fn element_insert_before(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        use boa_engine::JsArgs;
        let parent_id = get_node_id_from_this(this, context)?;
        let new_val = args.get_or_undefined(0);
        let new_id = get_node_id_from_this(new_val, context)?;
        let ref_val = args.get_or_undefined(1);
        let ref_id = if ref_val.is_null() || ref_val.is_undefined() {
            None
        } else {
            Some(get_node_id_from_this(ref_val, context)?)
        };
        let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
        if let Some(state) = state {
            let doc = state.borrow().document.clone();
            let doc_mut = doc.borrow_mut();
            let nodes = insertion_nodes(&doc_mut, parent_id, new_id)?;
            if let Some(ref_id) = ref_id {
                if doc_mut.parent(ref_id) != Some(parent_id) {
                    return Err(JsNativeError::typ()
                        .with_message("NotFoundError: reference is not a child")
                        .into());
                }
                if new_id == ref_id {
                    return Ok(new_val.clone());
                }
            }
            drop(doc_mut);
            let mut doc_mut = doc.borrow_mut();
            let has_observers = state.borrow().active_mutation_observers > 0;
            let (removals, disconnected) = if has_observers {
                let removals = capture_removals(&doc_mut, &nodes);
                (Some(removals), Vec::new())
            } else {
                let mut disconnected = Vec::new();
                for &node in &nodes {
                    if doc_mut.parent(node).is_some() {
                        disconnected.push(node);
                    }
                }
                (None, disconnected)
            };
            if let Some(ref_id) = ref_id {
                if new_id != ref_id {
                    for &node in &nodes {
                        doc_mut.insert_before(ref_id, node);
                    }
                }
            } else {
                for &node in &nodes {
                    doc_mut.append_child(parent_id, node);
                }
            }
            let previous = if has_observers {
                nodes
                    .first()
                    .and_then(|first| node_neighbors(&doc_mut, *first).0)
            } else {
                None
            };
            drop(doc_mut);
            if let Some(removals) = &removals {
                for removal in removals {
                    for &node in &removal.nodes {
                        super::custom_elements::notify_disconnected_subtree(node, context);
                    }
                }
            } else {
                for node in disconnected {
                    super::custom_elements::notify_disconnected_subtree(node, context);
                }
            }
            let owner = {
                let state_ref = state.borrow();
                state_ref
                    .owner_document(parent_id)
                    .unwrap_or(state_ref.main_document)
            };
            {
                let mut state_mut = state.borrow_mut();
                for &node in &nodes {
                    state_mut.set_subtree_owner_document(node, owner);
                }
            }
            if let Some(removals) = removals {
                record_removals(context, removals)?;
                record_child_mutation(context, parent_id, &nodes, &[], previous, ref_id)?;
            }
            for &node in &nodes {
                super::custom_elements::notify_connected_subtree(node, context)?;
            }
            super::script::prepare_connected_scripts(&nodes, context)?;
            super::iframe::prepare_connected_iframes(&nodes, context)?;
        }
        Ok(new_val.clone())
    })
}

pub(super) fn element_replace_child(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        use boa_engine::JsArgs;
        let parent_id = get_node_id_from_this(this, context)?;
        let new_val = args.get_or_undefined(0);
        let old_val = args.get_or_undefined(1);
        let new_id = get_node_id_from_this(new_val, context)?;
        let old_id = get_node_id_from_this(old_val, context)?;
        let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
        if let Some(state) = state {
            let doc = state.borrow().document.clone();
            let doc_mut = doc.borrow_mut();
            let nodes = insertion_nodes(&doc_mut, parent_id, new_id)?;
            if doc_mut.parent(old_id) != Some(parent_id) {
                return Err(JsNativeError::typ()
                    .with_message("NotFoundError: node is not a child")
                    .into());
            }
            if new_id != old_id {
                drop(doc_mut);
                let mut doc_mut = doc.borrow_mut();
                let has_observers = state.borrow().active_mutation_observers > 0;
                let (removals, previous, next) = if has_observers {
                    let removals = capture_removals(&doc_mut, &nodes);
                    let (p, n) = node_neighbors(&doc_mut, old_id);
                    (Some(removals), p, n)
                } else {
                    (None, None, None)
                };
                for &node in &nodes {
                    doc_mut.insert_before(old_id, node);
                }
                doc_mut.remove_child(old_id);
                drop(doc_mut);
                for &node in &nodes {
                    super::custom_elements::notify_disconnected_subtree(node, context);
                }
                super::custom_elements::notify_disconnected_subtree(old_id, context);
                let owner = {
                    let state_ref = state.borrow();
                    state_ref
                        .owner_document(parent_id)
                        .unwrap_or(state_ref.main_document)
                };
                {
                    let mut state_mut = state.borrow_mut();
                    for &node in &nodes {
                        state_mut.set_subtree_owner_document(node, owner);
                    }
                }
                if let Some(removals) = removals {
                    record_removals(context, removals)?;
                    record_child_mutation(context, parent_id, &nodes, &[old_id], previous, next)?;
                }
                for &node in &nodes {
                    super::custom_elements::notify_connected_subtree(node, context)?;
                }
                super::script::prepare_connected_scripts(&nodes, context)?;
                super::iframe::prepare_connected_iframes(&nodes, context)?;
            }
        }
        Ok(old_val.clone())
    })
}

pub(super) fn element_remove_self(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        let node_id = get_node_id_from_this(this, context)?;
        if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
            let doc = state.borrow().document.clone();
            let has_observers = state.borrow().active_mutation_observers > 0;
            let removals = if has_observers {
                Some(capture_removals(&doc.borrow(), &[node_id]))
            } else {
                None
            };
            doc.borrow_mut().remove_child(node_id);
            super::custom_elements::notify_disconnected_subtree(node_id, context);
            if let Some(removals) = removals {
                record_removals(context, removals)?;
            }
        }
        Ok(JsValue::undefined())
    })
}

pub(super) fn insertion_nodes(
    document: &Document,
    parent: NodeId,
    child: NodeId,
) -> JsResult<Vec<NodeId>> {
    validate_insertion(document, parent, child)?;
    let nodes = if matches!(document.get(child), Some(NodeData::DocumentFragment)) {
        document.children(child).collect()
    } else {
        vec![child]
    };
    for node in &nodes {
        validate_insertion(document, parent, *node)?;
    }
    Ok(nodes)
}

pub(super) fn validate_insertion(
    document: &Document,
    parent: NodeId,
    child: NodeId,
) -> JsResult<()> {
    if parent == child || document.ancestors(parent).any(|node| node == child) {
        return Err(JsNativeError::typ()
            .with_message("HierarchyRequestError: insertion creates a cycle")
            .into());
    }
    if !matches!(
        document.get(parent),
        Some(
            NodeData::Element(_)
                | NodeData::Document
                | NodeData::DocumentFragment
                | NodeData::ShadowRoot { .. },
        )
    ) {
        return Err(JsNativeError::typ()
            .with_message("HierarchyRequestError: parent cannot have children")
            .into());
    }
    Ok(())
}

pub(super) fn template_contents_or_self(document: &Document, node: NodeId) -> NodeId {
    match document.get(node) {
        Some(NodeData::Element(element)) => element.template_contents.unwrap_or(node),
        _ => node,
    }
}

pub(super) fn clone_subtree_into(
    source_document: &Document,
    source: NodeId,
    destination: &mut Document,
) -> NodeId {
    fn shallow(source_document: &Document, source: NodeId, destination: &mut Document) -> NodeId {
        let mut data = source_document
            .get(source)
            .cloned()
            .unwrap_or(NodeData::Text(String::new()));
        if let NodeData::Element(element) = &mut data {
            element.template_contents = element
                .template_contents
                .map(|_| destination.new_node(NodeData::DocumentFragment));
            element.shadow_root = None;
            element.canvas_bitmap = None;
            element.content_document = None;
        }
        destination.new_node(data)
    }
    let copy = shallow(source_document, source, destination);
    let mut pending = vec![(source, copy)];
    while let Some((source, parent)) = pending.pop() {
        let (source_fragment, copy_fragment, shadow_root_info) = {
            let source_elem = match source_document.get(source) {
                Some(NodeData::Element(e)) => Some((e.template_contents, e.shadow_root)),
                _ => None,
            };
            let copy_fragment = match destination.get(parent) {
                Some(NodeData::Element(e)) => e.template_contents,
                _ => None,
            };
            if let Some((src_frag, src_sr)) = source_elem {
                let sr_info = src_sr.and_then(|sr| {
                    if let Some(NodeData::ShadowRoot {
                        mode,
                        delegates_focus,
                        clonable,
                        ..
                    }) = source_document.get(sr)
                    {
                        Some((sr, mode.clone(), *delegates_focus, *clonable))
                    } else {
                        None
                    }
                });
                (src_frag, copy_fragment, sr_info)
            } else {
                (None, None, None)
            }
        };
        if let (Some(sf), Some(cf)) = (source_fragment, copy_fragment) {
            pending.push((sf, cf));
        }
        if let Some((src_root, mode, delegates_focus, clonable)) = shadow_root_info {
            let copy_root = destination.new_node(NodeData::ShadowRoot {
                host: parent,
                mode,
                delegates_focus,
                clonable,
            });
            if let Some(NodeData::Element(copy_elem)) = destination.get_mut(parent) {
                copy_elem.shadow_root = Some(copy_root);
            }
            pending.push((src_root, copy_root));
        }
        for child in source_document.children(source) {
            let copy = shallow(source_document, child, destination);
            destination.append_child(parent, copy);
            pending.push((child, copy));
        }
    }
    copy
}

pub(super) fn clone_subtree(document: &mut Document, source: NodeId, deep: bool) -> NodeId {
    fn shallow(document: &mut Document, source: NodeId) -> NodeId {
        let mut data = document
            .get(source)
            .cloned()
            .unwrap_or(NodeData::Text(String::new()));
        if let NodeData::Element(element) = &mut data {
            element.template_contents = element
                .template_contents
                .map(|_| document.new_node(NodeData::DocumentFragment));
            element.shadow_root = None;
            element.canvas_bitmap = None;
            element.content_document = None;
        }
        document.new_node(data)
    }
    let cloned = shallow(document, source);
    if deep {
        let mut pending = vec![(source, cloned)];
        while let Some((source, destination)) = pending.pop() {
            let (source_fragment, copy_fragment, shadow_root_info) = {
                let source_elem = match document.get(source) {
                    Some(NodeData::Element(e)) => Some((e.template_contents, e.shadow_root)),
                    _ => None,
                };
                let copy_fragment = match document.get(destination) {
                    Some(NodeData::Element(e)) => e.template_contents,
                    _ => None,
                };
                if let Some((src_frag, src_sr)) = source_elem {
                    let sr_info = src_sr.and_then(|sr| {
                        if let Some(NodeData::ShadowRoot {
                            mode,
                            delegates_focus,
                            clonable,
                            ..
                        }) = document.get(sr)
                        {
                            if *clonable {
                                Some((sr, mode.clone(), *delegates_focus))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    });
                    (src_frag, copy_fragment, sr_info)
                } else {
                    (None, None, None)
                }
            };
            if let (Some(sf), Some(cf)) = (source_fragment, copy_fragment) {
                pending.push((sf, cf));
            }
            if let Some((src_root, mode, delegates_focus)) = shadow_root_info {
                let copy_root = document.new_node(NodeData::ShadowRoot {
                    host: destination,
                    mode,
                    delegates_focus,
                    clonable: true,
                });
                if let Some(NodeData::Element(copy_elem)) = document.get_mut(destination) {
                    copy_elem.shadow_root = Some(copy_root);
                }
                pending.push((src_root, copy_root));
            }
            if document.children(source).next().is_some() {
                let children: Vec<_> = document.children(source).collect();
                for child in children {
                    let copy = shallow(document, child);
                    document.append_child(destination, copy);
                    pending.push((child, copy));
                }
            }
        }
    }
    cloned
}

pub(super) fn element_clone_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        use boa_engine::JsArgs;
        let node_id = get_node_id_from_this(this, context)?;
        let deep = args.get_or_undefined(0).to_boolean();
        let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
        let Some(state) = state else {
            return Ok(JsValue::null());
        };
        let doc = state.borrow().document.clone();
        let owner = {
            let state_ref = state.borrow();
            state_ref
                .owner_document(node_id)
                .unwrap_or(state_ref.main_document)
        };
        let source_scripts: Vec<NodeId> = {
            let doc_ref = doc.borrow();
            let is_script = |n: NodeId| {
                matches!(doc_ref.get(n), Some(NodeData::Element(el))
                    if el.name.ns.as_str() == "http://www.w3.org/1999/xhtml"
                        && el.name.local.as_str() == "script")
            };
            if !deep || doc_ref.children(node_id).next().is_none() {
                if is_script(node_id) {
                    vec![node_id]
                } else {
                    Vec::new()
                }
            } else {
                doc_ref.descendants(node_id).filter(|n| is_script(*n)).collect()
            }
        };
        let new_id = clone_subtree(&mut doc.borrow_mut(), node_id, deep);
        drop(doc);
        state.borrow_mut().set_subtree_owner_document(new_id, owner);
        super::custom_elements::upgrade_subtree(new_id, context)?;
        if !source_scripts.is_empty() {
            let state_ref = state.borrow();
            let any_started = source_scripts.iter().any(|n| state_ref.is_script_started(*n));
            drop(state_ref);
            if any_started {
                let doc = state.borrow().document.clone();
                let doc_ref = doc.borrow();
                let cloned_scripts: Vec<NodeId> = doc_ref
                    .descendants(new_id)
                    .filter(|n| {
                        matches!(doc_ref.get(*n), Some(NodeData::Element(el))
                            if el.name.ns.as_str() == "http://www.w3.org/1999/xhtml"
                                && el.name.local.as_str() == "script")
                    })
                    .collect();
                drop(doc_ref);
                let mut state_mut = state.borrow_mut();
                for (src, dest) in source_scripts.into_iter().zip(cloned_scripts) {
                    if state_mut.is_script_started(src) {
                        state_mut.mark_script_started(dest);
                    }
                }
            }
        }
        Ok(wrap_element(new_id, context).into())
    })
}

pub(super) fn element_dispatch_event(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let node_id = get_node_id_from_this(this, context)?;
    let event_val = args.get_or_undefined(0);
    let event_obj = event_val.as_object().ok_or_else(|| {
        JsNativeError::typ().with_message("dispatchEvent: argument must be an Event")
    })?;
    dispatch_existing_event_to_js(node_id, event_obj, context)
}
