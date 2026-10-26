use super::*;

fn notify_mutation_observer_change(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let delta = args.get_or_undefined(0).to_i32(context)?;
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>() {
        let mut state = state.borrow_mut();
        if delta > 0 {
            state.active_mutation_observers = state
                .active_mutation_observers
                .saturating_add(delta as usize);
        } else if delta < 0 {
            state.active_mutation_observers = state
                .active_mutation_observers
                .saturating_sub((-delta) as usize);
        }
    }
    Ok(JsValue::undefined())
}

pub(super) fn register_mutation_observer(context: &mut Context) -> JsResult<()> {
    let notify_fn = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(notify_mutation_observer_change),
    )
    .name(js_string!("__rixNotifyMutationObserverChange"))
    .length(1)
    .build();
    context.global_object().define_property_or_throw(
        js_string!("__rixNotifyMutationObserverChange"),
        PropertyDescriptor::builder()
            .value(notify_fn)
            .writable(true)
            .configurable(true)
            .enumerable(false),
        context,
    )?;

    crate::structured_clone::register_platform_objects(context, r#"
        (function(markNonCloneable) {
            const observers = new Set();
            const pending = new Set();
            let scheduled = false;
            class MutationRecord {
                constructor() { throw new TypeError('Illegal constructor: MutationRecord'); }
            }
            class MutationObserver {
                constructor(callback) {
                    if (typeof callback !== 'function') throw new TypeError('MutationObserver requires a callback');
                    markNonCloneable(this);
                    this._callback = callback;
                    this._registrations = new Map();
                    this._records = [];
                }
                observe(target, options) {
                    if (!(target instanceof Node)) throw new TypeError('MutationObserver target must be a Node');
                    const init = options || {};
                    const attributes = init.attributes === undefined ? init.attributeOldValue !== undefined || init.attributeFilter !== undefined : !!init.attributes;
                    const characterData = init.characterData === undefined ? init.characterDataOldValue !== undefined : !!init.characterData;
                    if ((!attributes && (init.attributeOldValue || init.attributeFilter !== undefined)) || (!characterData && init.characterDataOldValue) || (!attributes && !characterData && !init.childList)) {
                        throw new TypeError('MutationObserver options do not select mutation types');
                    }
                    this._registrations.set(target, {
                        attributes, characterData, childList: !!init.childList, subtree: !!init.subtree,
                        attributeOldValue: !!init.attributeOldValue, characterDataOldValue: !!init.characterDataOldValue,
                        attributeFilter: init.attributeFilter === undefined ? null : Array.from(init.attributeFilter, String),
                    });
                    if (!observers.has(this)) {
                        observers.add(this);
                        if (globalThis.__rixNotifyMutationObserverChange) __rixNotifyMutationObserverChange(1);
                    }
                }
                disconnect() {
                    this._registrations.clear();
                    this._records = [];
                    if (observers.delete(this)) {
                        if (globalThis.__rixNotifyMutationObserverChange) __rixNotifyMutationObserverChange(-1);
                    }
                    pending.delete(this);
                }
                takeRecords() {
                    const records = this._records;
                    this._records = [];
                    return records;
                }
            }
            globalThis.MutationObserver = MutationObserver;
            globalThis.MutationRecord = MutationRecord;
            Object.defineProperty(globalThis, '__rixRecordMutation', {configurable: true, value: function(source) {
                if (!observers.size) return;
                for (const observer of observers) {
                    let interested = false;
                    let keepOld = false;
                    for (const [target, options] of observer._registrations) {
                        if (target !== source.target && !(options.subtree && target.contains(source.target))) continue;
                        if (source.type === 'attributes' && (!options.attributes || (options.attributeFilter && !options.attributeFilter.includes(source.attributeName)))) continue;
                        if (source.type === 'characterData' && !options.characterData) continue;
                        if (source.type === 'childList' && !options.childList) continue;
                        interested = true;
                        keepOld = keepOld || (source.type === 'attributes' && options.attributeOldValue) || (source.type === 'characterData' && options.characterDataOldValue);
                    }
                    if (!interested) continue;
                    const record = Object.create(MutationRecord.prototype);
                    markNonCloneable(record);
                    for (const name of Object.keys(source)) {
                        Object.defineProperty(record, name, {value: name === 'oldValue' && !keepOld ? null : source[name], enumerable: true});
                    }
                    observer._records.push(record);
                    pending.add(observer);
                }
                if (pending.size && !scheduled) {
                    scheduled = true;
                    Promise.resolve().then(function() {
                        scheduled = false;
                        const batch = Array.from(pending);
                        pending.clear();
                        for (const observer of batch) {
                            const records = observer.takeRecords();
                            if (!records.length) continue;
                            try { observer._callback.call(observer, records, observer); }
                            catch (error) { if (globalThis.console) console.error(error); }
                        }
                    });
                }
            }});
        })
    "#)?;
    Ok(())
}

pub(super) struct DomMutation<'a> {
    pub(super) target: NodeId,
    pub(super) kind: &'static str,
    pub(super) attribute: Option<&'a str>,
    pub(super) old_value: Option<&'a str>,
    pub(super) added: &'a [NodeId],
    pub(super) removed: &'a [NodeId],
    pub(super) previous: Option<NodeId>,
    pub(super) next: Option<NodeId>,
}

pub(super) fn record_mutation(context: &mut Context, mutation: DomMutation<'_>) -> JsResult<()> {
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>() {
        if state.borrow().active_mutation_observers == 0 {
            return Ok(());
        }
    } else {
        return Ok(());
    }
    let function = context
        .global_object()
        .get(js_string!("__rixRecordMutation"), context)?;
    let Some(function) = function.as_callable() else {
        return Ok(());
    };
    let target = wrap_element(mutation.target, context);
    let added: Vec<JsValue> = mutation
        .added
        .iter()
        .map(|node| wrap_element(*node, context).into())
        .collect();
    let removed: Vec<JsValue> = mutation
        .removed
        .iter()
        .map(|node| wrap_element(*node, context).into())
        .collect();
    let added = boa_engine::object::builtins::JsArray::from_iter(added, context);
    let removed = boa_engine::object::builtins::JsArray::from_iter(removed, context);
    let previous = mutation
        .previous
        .map(|node| wrap_element(node, context).into())
        .unwrap_or(JsValue::null());
    let next = mutation
        .next
        .map(|node| wrap_element(node, context).into())
        .unwrap_or(JsValue::null());
    let record = ObjectInitializer::new(context)
        .property(
            js_string!("type"),
            js_string!(mutation.kind),
            Attribute::READONLY | Attribute::ENUMERABLE,
        )
        .property(
            js_string!("target"),
            target,
            Attribute::READONLY | Attribute::ENUMERABLE,
        )
        .property(
            js_string!("attributeName"),
            mutation
                .attribute
                .map(|name| js_string!(name).into())
                .unwrap_or(JsValue::null()),
            Attribute::READONLY | Attribute::ENUMERABLE,
        )
        .property(
            js_string!("attributeNamespace"),
            JsValue::null(),
            Attribute::READONLY | Attribute::ENUMERABLE,
        )
        .property(
            js_string!("oldValue"),
            mutation
                .old_value
                .map(|value| js_string!(value).into())
                .unwrap_or(JsValue::null()),
            Attribute::READONLY | Attribute::ENUMERABLE,
        )
        .property(
            js_string!("addedNodes"),
            added,
            Attribute::READONLY | Attribute::ENUMERABLE,
        )
        .property(
            js_string!("removedNodes"),
            removed,
            Attribute::READONLY | Attribute::ENUMERABLE,
        )
        .property(
            js_string!("previousSibling"),
            previous,
            Attribute::READONLY | Attribute::ENUMERABLE,
        )
        .property(
            js_string!("nextSibling"),
            next,
            Attribute::READONLY | Attribute::ENUMERABLE,
        )
        .build();
    function.call(&JsValue::undefined(), &[record.into()], context)?;
    Ok(())
}

pub(super) fn record_attribute_mutation(
    context: &mut Context,
    target: NodeId,
    name: &str,
    old: Option<&str>,
) -> JsResult<()> {
    record_mutation(
        context,
        DomMutation {
            target,
            kind: "attributes",
            attribute: Some(name),
            old_value: old,
            added: &[],
            removed: &[],
            previous: None,
            next: None,
        },
    )
}

pub(super) fn record_child_mutation(
    context: &mut Context,
    target: NodeId,
    added: &[NodeId],
    removed: &[NodeId],
    previous: Option<NodeId>,
    next: Option<NodeId>,
) -> JsResult<()> {
    if added.is_empty() && removed.is_empty() {
        return Ok(());
    }
    record_mutation(
        context,
        DomMutation {
            target,
            kind: "childList",
            attribute: None,
            old_value: None,
            added,
            removed,
            previous,
            next,
        },
    )
}

pub(super) fn node_neighbors(document: &Document, node: NodeId) -> (Option<NodeId>, Option<NodeId>) {
    let Some(parent) = document.parent(node) else {
        return (None, None);
    };
    let mut previous = None;
    let mut children = document.children(parent);
    while let Some(child) = children.next() {
        if child == node {
            return (previous, children.next());
        }
        previous = Some(child);
    }
    (None, None)
}

pub(super) struct RemovalMutation {
    pub(super) parent: NodeId,
    pub(super) nodes: Vec<NodeId>,
    pub(super) previous: Option<NodeId>,
    pub(super) next: Option<NodeId>,
}

pub(super) fn capture_removals(document: &Document, nodes: &[NodeId]) -> Vec<RemovalMutation> {
    let mut records = Vec::<RemovalMutation>::new();
    for &node in nodes {
        if let Some(parent) = document.parent(node) {
            let (previous, next) = node_neighbors(document, node);
            if matches!(document.get(parent), Some(NodeData::DocumentFragment)) {
                if let Some(record) = records.iter_mut().find(|record| record.parent == parent) {
                    record.nodes.push(node);
                    record.next = next;
                    continue;
                }
            }
            records.push(RemovalMutation {
                parent,
                nodes: vec![node],
                previous,
                next,
            });
        }
    }
    records
}

pub(super) fn record_removals(context: &mut Context, records: Vec<RemovalMutation>) -> JsResult<()> {
    for record in records {
        record_child_mutation(
            context,
            record.parent,
            &[],
            &record.nodes,
            record.previous,
            record.next,
        )?;
    }
    Ok(())
}
