
use super::*;
use std::cmp::Ordering;

#[derive(Clone, Copy)]
struct BoundaryPoint {
    container: NodeId,
    offset: u32,
}

#[derive(Clone, Copy)]
pub(super) struct RangeData {
    document: NodeId,
    start: BoundaryPoint,
    end: BoundaryPoint,
}

pub(super) fn register_range_interface(context: &mut Context) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!("Range"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Range constructor is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Range prototype is unavailable"))?;

    for (name, length, function) in [
        (
            "setStart",
            2,
            range_set_start as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
        ),
        ("setEnd", 2, range_set_end),
        ("setStartBefore", 1, range_set_start_before),
        ("setStartAfter", 1, range_set_start_after),
        ("setEndBefore", 1, range_set_end_before),
        ("setEndAfter", 1, range_set_end_after),
        ("collapse", 1, range_collapse),
        ("selectNode", 1, range_select_node),
        ("selectNodeContents", 1, range_select_node_contents),
        ("compareBoundaryPoints", 2, range_compare_boundary_points),
        ("comparePoint", 2, range_compare_point),
        ("isPointInRange", 2, range_is_point_in_range),
        ("intersectsNode", 1, range_intersects_node),
        ("cloneRange", 0, range_clone),
        ("detach", 0, range_detach),
        ("toString", 0, range_to_string),
        ("createContextualFragment", 1, range_create_contextual_fragment),
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
                .enumerable(true)
                .configurable(true)
                .build(),
            context,
        )?;
    }

    for (name, getter) in [
        (
            "startContainer",
            range_start_container as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
        ),
        ("startOffset", range_start_offset),
        ("endContainer", range_end_container),
        ("endOffset", range_end_offset),
        ("collapsed", range_collapsed),
        ("commonAncestorContainer", range_common_ancestor),
    ] {
        let getter =
            FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(getter))
                .name(format!("get {name}"))
                .build();
        prototype.define_property_or_throw(
            js_string!(name),
            PropertyDescriptor::builder()
                .get(getter)
                .enumerable(true)
                .configurable(true)
                .build(),
            context,
        )?;
    }

    for (name, value) in [
        ("START_TO_START", 0),
        ("START_TO_END", 1),
        ("END_TO_END", 2),
        ("END_TO_START", 3),
    ] {
        for object in [&constructor, &prototype] {
            object.define_property_or_throw(
                js_string!(name),
                PropertyDescriptor::builder()
                    .value(value)
                    .writable(false)
                    .enumerable(true)
                    .configurable(false)
                    .build(),
                context,
            )?;
        }
    }
    Ok(())
}

pub(super) fn document_create_range(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let document = document_root(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let constructor = context
        .global_object()
        .get(js_string!("Range"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Range constructor is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Range prototype is unavailable"))?;
    let range = ObjectInitializer::new(context).build();
    range.set_prototype(Some(prototype));
    crate::structured_clone::mark_platform_object(context, &range)?;
    state.borrow_mut().ranges.insert(
        range.clone(),
        RangeData {
            document,
            start: BoundaryPoint {
                container: document,
                offset: 0,
            },
            end: BoundaryPoint {
                container: document,
                offset: 0,
            },
        },
    );
    Ok(range.into())
}

fn range_and_state(
    this: &JsValue,
    context: &mut Context,
) -> JsResult<(Rc<RefCell<DomBridgeState>>, JsObject, RangeData)> {
    let object = this.as_object().ok_or_else(|| {
        JsNativeError::typ().with_message("Range method called on an incompatible receiver")
    })?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let data = state.borrow().ranges.get(&object).copied().ok_or_else(|| {
        JsNativeError::typ().with_message("Range method called on an incompatible receiver")
    })?;
    Ok((state, object, data))
}

fn range_argument(
    argument: Option<&JsValue>,
    context: &mut Context,
) -> JsResult<(Rc<RefCell<DomBridgeState>>, JsObject, RangeData)> {
    let object = argument
        .and_then(JsValue::as_object)
        .ok_or_else(|| JsNativeError::typ().with_message("A Range object is required"))?;
    let value: JsValue = object.clone().into();
    range_and_state(&value, context)
}

fn save_range(state: &Rc<RefCell<DomBridgeState>>, object: JsObject, data: RangeData) {
    state.borrow_mut().ranges.insert(object, data);
}

fn range_start_container(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (_, _, range) = range_and_state(this, context)?;
    Ok(wrap_element(range.start.container, context).into())
}

fn range_start_offset(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (_, _, range) = range_and_state(this, context)?;
    Ok(JsValue::new(range.start.offset))
}

fn range_end_container(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (_, _, range) = range_and_state(this, context)?;
    Ok(wrap_element(range.end.container, context).into())
}

fn range_end_offset(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (_, _, range) = range_and_state(this, context)?;
    Ok(JsValue::new(range.end.offset))
}

fn range_collapsed(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (_, _, range) = range_and_state(this, context)?;
    Ok(JsValue::new(
        range.start.container == range.end.container && range.start.offset == range.end.offset,
    ))
}

fn range_common_ancestor(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, _, range) = range_and_state(this, context)?;
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let node = common_ancestor(&document, range.start.container, range.end.container)
        .unwrap_or(range.document);
    drop(document);
    Ok(wrap_element(node, context).into())
}

fn range_set_start(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    set_range_boundary(this, args, true, context)
}

fn range_set_end(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    set_range_boundary(this, args, false, context)
}

fn set_range_boundary(
    this: &JsValue,
    args: &[JsValue],
    is_start: bool,
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, object, mut range) = range_and_state(this, context)?;
    let node = node_argument(args.first(), context)?;
    let offset = unsigned_long(args.get_or_undefined(1), context)?;
    validate_boundary(&state, range.document, node, offset, context)?;
    let boundary = BoundaryPoint {
        container: node,
        offset,
    };
    let document = state.borrow().document.clone();
    let document = document.borrow();
    if is_start {
        range.start = boundary;
        match compare_points(&document, range.start, range.end) {
            Some(Ordering::Greater) | None => range.end = boundary,
            _ => {}
        }
    } else {
        range.end = boundary;
        match compare_points(&document, range.start, range.end) {
            Some(Ordering::Greater) | None => range.start = boundary,
            _ => {}
        }
    }
    drop(document);
    save_range(&state, object, range);
    Ok(JsValue::undefined())
}

fn node_argument(argument: Option<&JsValue>, context: &mut Context) -> JsResult<NodeId> {
    let value =
        argument.ok_or_else(|| JsNativeError::typ().with_message("A Node argument is required"))?;
    let object = value
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("A Node argument is required"))?;
    let node = get_node_id_from_this(value, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    if !state
        .borrow()
        .element_wrappers
        .get(&node)
        .is_some_and(|wrapper| JsObject::equals(wrapper, &object))
    {
        return Err(JsNativeError::typ()
            .with_message("A genuine Node argument is required")
            .into());
    }
    Ok(node)
}

fn unsigned_long(value: &JsValue, context: &mut Context) -> JsResult<u32> {
    let number = value.to_number(context)?;
    if !number.is_finite() || number == 0.0 {
        return Ok(0);
    }
    let modulo = (number.trunc() % 4_294_967_296.0 + 4_294_967_296.0) % 4_294_967_296.0;
    Ok(modulo as u32)
}

fn validate_boundary(
    state: &Rc<RefCell<DomBridgeState>>,
    range_document: NodeId,
    node: NodeId,
    offset: u32,
    context: &mut Context,
) -> JsResult<()> {
    let (owner, length, invalid_container) = {
        let bridge = state.borrow();
        let document = bridge.document.clone();
        let document = document.borrow();
        let owner = document_for_node(&bridge, &document, node);
        let invalid_container = matches!(document.get(node), Some(NodeData::DocumentType { .. }));
        let length = boundary_length(&document, node);
        (owner, length, invalid_container)
    };
    if invalid_container {
        return Err(range_dom_exception(
            "InvalidNodeTypeError",
            "DocumentType is not a Range boundary container",
            context,
        )?);
    }
    if owner != Some(range_document) {
        return Err(range_dom_exception(
            "WrongDocumentError",
            "Range boundary belongs to a different document",
            context,
        )?);
    }
    if offset as usize > length {
        return Err(range_dom_exception(
            "IndexSizeError",
            "Range offset exceeds the node length",
            context,
        )?);
    }
    Ok(())
}

fn document_for_node(bridge: &DomBridgeState, document: &Document, node: NodeId) -> Option<NodeId> {
    if matches!(document.get(node), Some(NodeData::Document)) {
        Some(node)
    } else {
        bridge.owner_document(node).or(Some(bridge.main_document))
    }
}

fn boundary_length(document: &Document, node: NodeId) -> usize {
    match document.get(node) {
        Some(NodeData::Text(value) | NodeData::Comment(value)) => value.encode_utf16().count(),
        Some(NodeData::DocumentType { .. }) | None => 0,
        _ => document.children(node).count(),
    }
}

fn range_set_start_before(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    set_boundary_before_after(this, args, true, false, context)
}

fn range_set_start_after(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    set_boundary_before_after(this, args, true, true, context)
}

fn range_set_end_before(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    set_boundary_before_after(this, args, false, false, context)
}

fn range_set_end_after(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    set_boundary_before_after(this, args, false, true, context)
}

fn set_boundary_before_after(
    this: &JsValue,
    args: &[JsValue],
    is_start: bool,
    after: bool,
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, object, range) = range_and_state(this, context)?;
    let node = node_argument(args.first(), context)?;
    let (parent, offset) = {
        let bridge = state.borrow();
        let document = bridge.document.clone();
        let document = document.borrow();
        let parent = document.parent(node).ok_or_else(|| {
            JsNativeError::typ().with_message("The node has no parent for a boundary point")
        })?;
        let index = document
            .children(parent)
            .position(|child| child == node)
            .ok_or_else(|| JsNativeError::error().with_message("Node parent invariant violated"))?;
        (parent, index as u32 + u32::from(after))
    };
    validate_boundary(&state, range.document, parent, offset, context)?;
    let mut updated = range;
    let point = BoundaryPoint {
        container: parent,
        offset,
    };
    let document = state.borrow().document.clone();
    let document = document.borrow();
    if is_start {
        updated.start = point;
        if compare_points(&document, updated.start, updated.end) != Some(Ordering::Less) {
            updated.end = point;
        }
    } else {
        updated.end = point;
        if compare_points(&document, updated.start, updated.end) == Some(Ordering::Greater) {
            updated.start = point;
        }
    }
    drop(document);
    save_range(&state, object, updated);
    Ok(JsValue::undefined())
}

fn range_collapse(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (state, object, mut range) = range_and_state(this, context)?;
    let to_start = args.get_or_undefined(0).to_boolean();
    if to_start {
        range.end = range.start;
    } else {
        range.start = range.end;
    }
    save_range(&state, object, range);
    Ok(JsValue::undefined())
}

fn range_select_node(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (state, object, range) = range_and_state(this, context)?;
    let node = node_argument(args.first(), context)?;
    let parent = {
        let bridge = state.borrow();
        let document = bridge.document.clone();
        let document = document.borrow();
        let parent = document.parent(node).ok_or_else(|| {
            JsNativeError::typ().with_message("selectNode requires a node with a parent")
        })?;
        let index = document
            .children(parent)
            .position(|child| child == node)
            .ok_or_else(|| JsNativeError::error().with_message("Node parent invariant violated"))?;
        (parent, index as u32)
    };
    validate_boundary(&state, range.document, parent.0, parent.1 + 1, context)?;
    save_range(
        &state,
        object,
        RangeData {
            document: range.document,
            start: BoundaryPoint {
                container: parent.0,
                offset: parent.1,
            },
            end: BoundaryPoint {
                container: parent.0,
                offset: parent.1 + 1,
            },
        },
    );
    Ok(JsValue::undefined())
}

fn range_select_node_contents(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, object, range) = range_and_state(this, context)?;
    let node = node_argument(args.first(), context)?;
    let end = {
        let bridge = state.borrow();
        let document = bridge.document.clone();
        let document = document.borrow();
        boundary_length(&document, node) as u32
    };
    validate_boundary(&state, range.document, node, end, context)?;
    save_range(
        &state,
        object,
        RangeData {
            document: range.document,
            start: BoundaryPoint {
                container: node,
                offset: 0,
            },
            end: BoundaryPoint {
                container: node,
                offset: end,
            },
        },
    );
    Ok(JsValue::undefined())
}

fn range_compare_boundary_points(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, _, range) = range_and_state(this, context)?;
    let (_, _, source) = range_argument(args.get(1), context)?;
    if range.document != source.document {
        return Err(range_dom_exception(
            "WrongDocumentError",
            "Ranges belong to different documents",
            context,
        )?);
    }
    let mode = unsigned_short(args.get_or_undefined(0), context)?;
    let (left, right) = match mode {
        0 => (range.start, source.start),
        1 => (range.start, source.end),
        2 => (range.end, source.end),
        3 => (range.end, source.start),
        _ => {
            return Err(JsNativeError::typ()
                .with_message("Invalid Range boundary comparison mode")
                .into());
        }
    };
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let order = compare_points(&document, left, right).unwrap_or(Ordering::Equal);
    Ok(JsValue::new(match order {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }))
}

fn unsigned_short(value: &JsValue, context: &mut Context) -> JsResult<u16> {
    let number = value.to_number(context)?;
    if !number.is_finite() || number == 0.0 {
        return Ok(0);
    }
    let modulo = (number.trunc() % 65_536.0 + 65_536.0) % 65_536.0;
    Ok(modulo as u16)
}

fn range_compare_point(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, _, range) = range_and_state(this, context)?;
    let node = node_argument(args.first(), context)?;
    let offset = unsigned_long(args.get_or_undefined(1), context)?;
    validate_boundary(&state, range.document, node, offset, context)?;
    let point = BoundaryPoint {
        container: node,
        offset,
    };
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let before = compare_points(&document, point, range.start).unwrap_or(Ordering::Equal);
    let after = compare_points(&document, point, range.end).unwrap_or(Ordering::Equal);
    Ok(JsValue::new(if before == Ordering::Less {
        -1
    } else if after == Ordering::Greater {
        1
    } else {
        0
    }))
}

fn range_is_point_in_range(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, _, range) = range_and_state(this, context)?;
    let node = node_argument(args.first(), context)?;
    if document_for_node(&state.borrow(), &state.borrow().document.borrow(), node)
        != Some(range.document)
    {
        return Ok(JsValue::new(false));
    }
    let offset = unsigned_long(args.get_or_undefined(1), context)?;
    validate_boundary(&state, range.document, node, offset, context)?;
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let point = BoundaryPoint {
        container: node,
        offset,
    };
    let after_start =
        compare_points(&document, point, range.start).is_some_and(|v| v != Ordering::Less);
    let before_end =
        compare_points(&document, point, range.end).is_some_and(|v| v != Ordering::Greater);
    Ok(JsValue::new(after_start && before_end))
}

fn range_intersects_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, _, range) = range_and_state(this, context)?;
    let node = node_argument(args.first(), context)?;
    if document_for_node(&state.borrow(), &state.borrow().document.borrow(), node)
        != Some(range.document)
    {
        return Ok(JsValue::new(false));
    }
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let Some(parent) = document.parent(node) else {
        return Ok(JsValue::new(node == range.document));
    };
    let Some(index) = document.children(parent).position(|child| child == node) else {
        return Ok(JsValue::new(false));
    };
    let before = BoundaryPoint {
        container: parent,
        offset: index as u32,
    };
    let after = BoundaryPoint {
        container: parent,
        offset: index as u32 + 1,
    };
    let overlaps_start =
        compare_points(&document, range.end, before).is_some_and(|v| v == Ordering::Greater);
    let overlaps_end =
        compare_points(&document, range.start, after).is_some_and(|v| v == Ordering::Less);
    Ok(JsValue::new(overlaps_start && overlaps_end))
}

fn range_clone(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (state, _, data) = range_and_state(this, context)?;
    let constructor = context
        .global_object()
        .get(js_string!("Range"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Range constructor is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Range prototype is unavailable"))?;
    let range = ObjectInitializer::new(context).build();
    range.set_prototype(Some(prototype));
    crate::structured_clone::mark_platform_object(context, &range)?;
    state.borrow_mut().ranges.insert(range.clone(), data);
    Ok(range.into())
}

fn range_detach(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    range_and_state(this, context)?;
    Ok(JsValue::undefined())
}

fn range_to_string(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (state, _, range) = range_and_state(this, context)?;
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let root = tree_root(&document, range.start.container);
    let nodes = ordered_subtree(&document, root);
    let mut text = String::new();
    for node in nodes {
        let Some(NodeData::Text(value)) = document.get(node) else {
            continue;
        };
        let length = value.encode_utf16().count() as u32;
        let start = BoundaryPoint {
            container: node,
            offset: 0,
        };
        let end = BoundaryPoint {
            container: node,
            offset: length,
        };
        let Some(range_before_end) = compare_points(&document, range.start, end) else {
            continue;
        };
        let Some(range_after_start) = compare_points(&document, range.end, start) else {
            continue;
        };
        if range_before_end != Ordering::Less || range_after_start != Ordering::Greater {
            continue;
        }
        let start_offset = if range.start.container == node {
            range.start.offset
        } else {
            0
        };
        let end_offset = if range.end.container == node {
            range.end.offset
        } else {
            length
        };
        if start_offset < end_offset {
            text.push_str(&utf16_slice(value, start_offset, end_offset));
        }
    }
    Ok(js_string!(text).into())
}

fn utf16_slice(value: &str, start: u32, end: u32) -> String {
    let units = value.encode_utf16().collect::<Vec<_>>();
    let start = (start as usize).min(units.len());
    let end = (end as usize).max(start).min(units.len());
    String::from_utf16_lossy(&units[start..end])
}

fn ordered_subtree(document: &Document, root: NodeId) -> Vec<NodeId> {
    let mut nodes = Vec::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        nodes.push(node);
        pending.extend(
            document
                .children(node)
                .collect::<Vec<_>>()
                .into_iter()
                .rev(),
        );
    }
    nodes
}

fn tree_root(document: &Document, node: NodeId) -> NodeId {
    let mut root = node;
    while let Some(parent) = document.parent(root) {
        root = parent;
    }
    root
}

fn common_ancestor(document: &Document, left: NodeId, right: NodeId) -> Option<NodeId> {
    let left = ancestor_path(document, left);
    let right = ancestor_path(document, right);
    left.iter()
        .rev()
        .zip(right.iter().rev())
        .take_while(|(a, b)| a == b)
        .last()
        .map(|(node, _)| *node)
}

fn ancestor_path(document: &Document, node: NodeId) -> Vec<NodeId> {
    let mut path = vec![node];
    let mut current = node;
    while let Some(parent) = document.parent(current) {
        path.push(parent);
        current = parent;
    }
    path
}

fn compare_points(
    document: &Document,
    left: BoundaryPoint,
    right: BoundaryPoint,
) -> Option<Ordering> {
    if left.container == right.container {
        return Some(left.offset.cmp(&right.offset));
    }
    let left_path = ancestor_path(document, left.container);
    let right_path = ancestor_path(document, right.container);
    let mut left_root_path = left_path.iter().rev();
    let mut right_root_path = right_path.iter().rev();
    let mut common_depth = 0usize;
    loop {
        match (left_root_path.next(), right_root_path.next()) {
            (Some(a), Some(b)) if a == b => common_depth += 1,
            (Some(_), Some(_)) => break,
            (None, None) => break,
            (Some(_), None) | (None, Some(_)) => break,
        }
    }
    if common_depth == left_path.len() {
        let child = *right_path.get(right_path.len().checked_sub(common_depth + 1)?)?;
        let parent = left.container;
        let index = document
            .children(parent)
            .position(|candidate| candidate == child)? as u32;
        return Some(if left.offset <= index {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    if common_depth == right_path.len() {
        let child = *left_path.get(left_path.len().checked_sub(common_depth + 1)?)?;
        let parent = right.container;
        let index = document
            .children(parent)
            .position(|candidate| candidate == child)? as u32;
        return Some(if right.offset <= index {
            Ordering::Greater
        } else {
            Ordering::Less
        });
    }
    let left_child = *left_path.get(left_path.len().checked_sub(common_depth + 1)?)?;
    let right_child = *right_path.get(right_path.len().checked_sub(common_depth + 1)?)?;
    let ancestor = *left_path.get(left_path.len().checked_sub(common_depth)?)?;
    let left_index = document
        .children(ancestor)
        .position(|child| child == left_child)?;
    let right_index = document
        .children(ancestor)
        .position(|child| child == right_child)?;
    Some(left_index.cmp(&right_index))
}

fn range_dom_exception(
    message_name: &str,
    message: &str,
    context: &mut Context,
) -> JsResult<JsError> {
    let constructor = context
        .global_object()
        .get(js_string!("DOMException"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("DOMException constructor is unavailable")
        })?;
    let exception = constructor.construct(
        &[js_string!(message).into(), js_string!(message_name).into()],
        None,
        context,
    )?;
    Ok(JsError::from_opaque(exception.into()))
}

fn range_create_contextual_fragment(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        use boa_engine::JsArgs;
        let (state, _, range) = range_and_state(this, context)?;
        let start_node = range.start.container;
        let document_root = range.document;

        let context_qual_name = {
            let doc = state.borrow().document.clone();
            let doc_ref = doc.borrow();
            if let Some(NodeData::Element(elem)) = doc_ref.get(start_node) {
                Some(elem.name.clone())
            } else if matches!(
                doc_ref.get(start_node),
                Some(NodeData::Text(_))
                    | Some(NodeData::Comment(_))
                    | Some(NodeData::DocumentType { .. })
            ) {
                doc_ref.parent(start_node).and_then(|parent| {
                    if let Some(NodeData::Element(parent_elem)) = doc_ref.get(parent) {
                        Some(parent_elem.name.clone())
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        };

        let markup = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();

        let (parsed_doc, source_nodes) =
            rix_html::parse_html_fragment(&markup, context_qual_name.as_ref());

        let doc = state.borrow().document.clone();
        let fragment = doc.borrow_mut().new_node(NodeData::DocumentFragment);
        state
            .borrow_mut()
            .set_subtree_owner_document(fragment, document_root);

        let mut added_nodes = Vec::new();
        {
            let mut doc_mut = doc.borrow_mut();
            for source_child in source_nodes {
                let cloned = clone_subtree_into(&parsed_doc, source_child, &mut doc_mut);
                doc_mut.append_child(fragment, cloned);
                added_nodes.push(cloned);
            }
        }

        for added in &added_nodes {
            state
                .borrow_mut()
                .set_subtree_owner_document(*added, document_root);
        }

        {
            let mut state_mut = state.borrow_mut();
            let doc_ref = state_mut.document.clone();
            let doc_ref = doc_ref.borrow();
            for root_node in &added_nodes {
                for n in doc_ref.descendants(*root_node) {
                    if let Some(NodeData::Element(el)) = doc_ref.get(n) {
                        if el.name.ns.as_str() == "http://www.w3.org/1999/xhtml"
                            && el.name.local.as_str() == "script"
                        {
                            state_mut.mark_script_started(n);
                        }
                    }
                }
            }
        }

        Ok(wrap_element(fragment, context).into())
    })
}

