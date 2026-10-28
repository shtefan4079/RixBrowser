
use super::*;

fn attribute_may_affect_style(name: &str) -> bool {
    !matches!(
        name,
        "aria-label"
            | "aria-describedby"
            | "aria-labelledby"
            | "aria-live"
            | "aria-atomic"
            | "aria-busy"
            | "aria-relevant"
            | "aria-keyshortcuts"
            | "aria-roledescription"
            | "aria-valuenow"
            | "aria-valuemin"
            | "aria-valuemax"
            | "aria-valuetext"
            | "title"
            | "alt"
    )
}

pub(super) fn element_get_attribute_records(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let records = {
        let state_ref = state.borrow();
        let document = state_ref.document.borrow();
        match document.get(node_id) {
            Some(NodeData::Element(element)) => element
                .attributes
                .iter()
                .map(|(name, value)| {
                    let local_name = name.local.to_string();
                    let prefix = name.prefix.as_ref().map(ToString::to_string);
                    let qualified_name = prefix.as_ref().map_or_else(
                        || local_name.clone(),
                        |prefix| format!("{prefix}:{local_name}"),
                    );
                    (
                        qualified_name,
                        local_name,
                        name.ns.to_string(),
                        prefix,
                        value.clone(),
                    )
                })
                .collect::<Vec<_>>(),
            _ => {
                return Err(JsNativeError::typ()
                    .with_message("Element attribute access requires an Element receiver")
                    .into())
            }
        }
    };

    let mut values = Vec::with_capacity(records.len());
    for (name, local_name, namespace_uri, prefix, value) in records {
        let mut record = ObjectInitializer::new(context);
        record.property(js_string!("name"), js_string!(name), Attribute::READONLY);
        record.property(
            js_string!("localName"),
            js_string!(local_name),
            Attribute::READONLY,
        );
        record.property(
            js_string!("namespaceURI"),
            if namespace_uri.is_empty() {
                JsValue::null()
            } else {
                js_string!(namespace_uri).into()
            },
            Attribute::READONLY,
        );
        record.property(
            js_string!("prefix"),
            prefix.map_or(JsValue::null(), |prefix| js_string!(prefix).into()),
            Attribute::READONLY,
        );
        record.property(js_string!("value"), js_string!(value), Attribute::READONLY);
        values.push(record.build().into());
    }
    Ok(boa_engine::object::builtins::JsArray::from_iter(values, context).into())
}

pub(super) fn element_get_attribute_ns(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let namespace_uri = namespace_argument(args.get_or_undefined(0), context)?;
    let local_name = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return Ok(JsValue::null());
    };
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let value = match document.get(node_id) {
        Some(NodeData::Element(element)) => element.get_attribute_ns(&namespace_uri, &local_name),
        _ => {
            return Err(JsNativeError::typ()
                .with_message("Element method requires an Element receiver")
                .into())
        }
    };
    Ok(value.map_or(JsValue::null(), |value| js_string!(value).into()))
}

pub(super) fn element_set_attribute_record(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        let node_id = get_node_id_from_this(this, context)?;
        let namespace_uri = namespace_argument(args.get_or_undefined(0), context)?;
        let prefix = if args.get_or_undefined(1).is_null() || args.get_or_undefined(1).is_undefined() {
            None
        } else {
            Some(
                args.get_or_undefined(1)
                    .to_string(context)?
                    .to_std_string_escaped(),
            )
        };
        let local_name = args
            .get_or_undefined(2)
            .to_string(context)?
            .to_std_string_escaped();
        let value = args
            .get_or_undefined(3)
            .to_string(context)?
            .to_std_string_escaped();
        let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
            return Ok(JsValue::undefined());
        };
        let document = state.borrow().document.clone();
        let previous = {
            let mut document = document.borrow_mut();
            match document.get_mut_untracked(node_id) {
                Some(NodeData::Element(element)) => {
                    element.set_attribute_ns(&namespace_uri, prefix.as_deref(), &local_name, &value)
                }
                _ => {
                    return Err(JsNativeError::typ()
                        .with_message("Element method requires an Element receiver")
                        .into())
                }
            }
        };
        canvas::canvas_dimension_attribute_mutated(context, node_id, &namespace_uri, &local_name)?;
        let old_value = previous.as_ref().map(|(old_value, _)| old_value.as_str());
        let changed = previous.as_ref().map_or(true, |(old_value, old_prefix)| {
            old_value != &value || old_prefix.as_deref() != prefix.as_deref()
        });
        if changed {
            {
                let mut doc = document.borrow_mut();
                if doc.is_connected(node_id) {
                    doc.bump_revision();
                    if attribute_may_affect_style(&local_name) {
                        doc.bump_style_revision();
                    }
                    if matches!(doc.get(node_id), Some(NodeData::Element(e)) if e.name.local.as_str() == "script") {
                        doc.bump_script_revision();
                    }
                }
            }
            notify_attribute_mutation(this, &namespace_uri, &local_name, Some(&value), context)?;
            record_attribute_mutation(context, node_id, &local_name, old_value)?;
            super::custom_elements::notify_attribute_changed(
                node_id,
                &local_name,
                old_value,
                Some(&value),
                context,
            );
        }
        Ok(JsValue::undefined())
    })
}

pub(super) fn element_remove_attribute_ns(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    super::custom_elements::with_ce_reactions(context, |context| {
        let node_id = get_node_id_from_this(this, context)?;
        let namespace_uri = namespace_argument(args.get_or_undefined(0), context)?;
        let local_name = args
            .get_or_undefined(1)
            .to_string(context)?
            .to_std_string_escaped();
        let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
            return Ok(JsValue::undefined());
        };
        let document = state.borrow().document.clone();
        let old_value = {
            let mut document = document.borrow_mut();
            match document.get_mut_untracked(node_id) {
                Some(NodeData::Element(element)) => {
                    element.remove_attribute_ns(&namespace_uri, &local_name)
                }
                _ => {
                    return Err(JsNativeError::typ()
                        .with_message("Element method requires an Element receiver")
                        .into())
                }
            }
        };
        if let Some(old_value) = old_value {
            {
                let mut doc = document.borrow_mut();
                if doc.is_connected(node_id) {
                    doc.bump_revision();
                    if attribute_may_affect_style(&local_name) {
                        doc.bump_style_revision();
                    }
                    if matches!(doc.get(node_id), Some(NodeData::Element(e)) if e.name.local.as_str() == "script") {
                        doc.bump_script_revision();
                    }
                }
            }
            canvas::canvas_dimension_attribute_mutated(context, node_id, &namespace_uri, &local_name)?;
            notify_attribute_mutation(this, &namespace_uri, &local_name, None, context)?;
            record_attribute_mutation(context, node_id, &local_name, Some(&old_value))?;
            super::custom_elements::notify_attribute_changed(
                node_id,
                &local_name,
                Some(&old_value),
                None,
                context,
            );
        }
        Ok(JsValue::undefined())
    })
}

pub(super) fn element_has_attribute_ns(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let namespace_uri = namespace_argument(args.get_or_undefined(0), context)?;
    let local_name = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return Ok(JsValue::new(false));
    };
    let document = state.borrow().document.clone();
    let document = document.borrow();
    match document.get(node_id) {
        Some(NodeData::Element(element)) => Ok(JsValue::new(
            element
                .get_attribute_ns(&namespace_uri, &local_name)
                .is_some(),
        )),
        _ => Err(JsNativeError::typ()
            .with_message("Element method requires an Element receiver")
            .into()),
    }
}

fn namespace_argument(value: &JsValue, context: &mut Context) -> JsResult<String> {
    if value.is_null() || value.is_undefined() {
        Ok(String::new())
    } else {
        Ok(value.to_string(context)?.to_std_string_escaped())
    }
}

pub(super) fn notify_attribute_mutation(
    element: &JsValue,
    namespace_uri: &str,
    local_name: &str,
    value: Option<&str>,
    context: &mut Context,
) -> JsResult<()> {
    let callback = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|state| state.borrow().attribute_mutation_callback.clone());
    if let Some(callback) = callback {
        let args = [
            element.clone(),
            if namespace_uri.is_empty() {
                JsValue::null()
            } else {
                js_string!(namespace_uri).into()
            },
            js_string!(local_name).into(),
            value.map_or(JsValue::null(), |value| js_string!(value).into()),
        ];
        callback.call(&JsValue::undefined(), &args, context)?;
    }
    Ok(())
}

pub(super) fn register_attribute_interfaces(context: &mut Context) -> JsResult<()> {
    let callback = context.eval(boa_engine::Source::from_bytes(
        r#"
        (function() {
            function interfaceConstructor(name, parent) {
                const Constructor = function() {
                    if (!new.target) throw new TypeError(name + ' requires new');
                    throw new TypeError('Illegal constructor: ' + name);
                };
                Object.defineProperty(Constructor, 'name', {value: name});
                Constructor.prototype = Object.create(parent.prototype, {
                    constructor: {value: Constructor, writable: true, configurable: true},
                    [Symbol.toStringTag]: {value: name, configurable: true},
                });
                Object.setPrototypeOf(Constructor, parent);
                globalThis[name] = Constructor;
                return Constructor;
            }

            const Attr = interfaceConstructor('Attr', Node);
            const NamedNodeMap = interfaceConstructor('NamedNodeMap', Object);
            const attrStates = new WeakMap();
            const attrCaches = new WeakMap();
            const mapOwners = new WeakMap();
            const emptyChildNodes = Object.freeze([]);

            function attrState(attr) {
                const state = attrStates.get(attr);
                if (!state) throw new TypeError('Illegal invocation');
                return state;
            }
            function mapOwner(map) {
                const owner = mapOwners.get(map);
                if (!owner) throw new TypeError('Illegal invocation');
                return owner;
            }
            function stateKey(namespaceURI, localName) {
                return (namespaceURI || '') + '\\u0000' + localName;
            }
            function namespaceOrNull(namespaceURI) {
                if (namespaceURI == null) return null;
                const value = String(namespaceURI);
                return value === '' ? null : value;
            }
            function cacheFor(element) {
                let cache = attrCaches.get(element);
                if (!cache) {
                    cache = new Map();
                    attrCaches.set(element, cache);
                }
                return cache;
            }
            function makeAttr(record, ownerElement, ownerDocument) {
                const attr = Object.create(Attr.prototype);
                attrStates.set(attr, {
                    name: record.name,
                    localName: record.localName,
                    namespaceURI: record.namespaceURI,
                    prefix: record.prefix,
                    value: record.value,
                    ownerElement,
                    ownerDocument,
                });
                return attr;
            }
            function attrFromRecord(element, record) {
                const key = stateKey(record.namespaceURI, record.localName);
                const cache = cacheFor(element);
                const existing = cache.get(key);
                if (existing) {
                    const state = attrState(existing);
                    if (state.ownerElement === element) {
                        state.name = record.name;
                        state.localName = record.localName;
                        state.namespaceURI = record.namespaceURI;
                        state.prefix = record.prefix;
                        state.value = record.value;
                        return existing;
                    }
                }
                const attr = makeAttr(record, element, element.ownerDocument);
                cache.set(key, attr);
                return attr;
            }
            function records(element) { return element.__getAttributeRecords(); }
            function canonicalName(element, name) {
                return element.namespaceURI === 'http://www.w3.org/1999/xhtml'
                    ? name.replace(/[A-Z]/g, ch => ch.toLowerCase()) : name;
            }
            function findByName(element, name) {
                const expected = canonicalName(element, String(name));
                return records(element).find(record => record.name === expected) || null;
            }
            function attributeKey(state) { return stateKey(state.namespaceURI, state.localName); }
            function throwNotFound() { throw new DOMException('The requested attribute was not found', 'NotFoundError'); }
            function setAttrValue(attr, value) {
                const state = attrState(attr);
                const text = String(value);
                if (state.ownerElement) {
                    state.ownerElement.__setAttributeRecord(
                        state.namespaceURI, state.prefix, state.localName, text);
                } else {
                    state.value = text;
                }
            }
            function removeAttrRecord(element, state) {
                if (state.namespaceURI == null) element.removeAttribute(state.name);
                else element.removeAttributeNS(state.namespaceURI, state.localName);
            }
            function setNamedItem(map, attr, byNamespace) {
                const element = mapOwner(map);
                const state = attrStates.get(attr);
                if (!state) throw new TypeError("The argument is not of type 'Attr'");
                if (state.ownerElement !== null) {
                    throw new DOMException('The attribute is already in use', 'InUseAttributeError');
                }
                if (state.ownerDocument && element.ownerDocument &&
                    state.ownerDocument !== element.ownerDocument) {
                    throw new DOMException('The attribute belongs to another document', 'WrongDocumentError');
                }
                const existingRecord = byNamespace
                    ? records(element).find(record => record.namespaceURI === state.namespaceURI && record.localName === state.localName)
                    : findByName(element, state.name);
                let replaced = null;
                if (existingRecord) {
                    replaced = attrFromRecord(element, existingRecord);
                    const oldState = attrState(replaced);
                    removeAttrRecord(element, oldState);
                    cacheFor(element).delete(attributeKey(oldState));
                }
                element.__setAttributeRecord(
                    state.namespaceURI, state.prefix, state.localName, state.value);
                state.ownerElement = element;
                state.ownerDocument = element.ownerDocument;
                cacheFor(element).set(attributeKey(state), attr);
                return replaced;
            }
            function removeNamedItem(map, name, byNamespace, namespaceURI) {
                const element = mapOwner(map);
                const record = byNamespace
                    ? records(element).find(item => item.namespaceURI === namespaceURI && item.localName === String(name)) || null
                    : findByName(element, name);
                if (!record) return throwNotFound();
                const attr = attrFromRecord(element, record);
                const state = attrState(attr);
                removeAttrRecord(element, state);
                cacheFor(element).delete(attributeKey(state));
                return attr;
            }
            function indexFromKey(key) {
                if (typeof key !== 'string' || !/^(0|[1-9][0-9]*)$/.test(key)) return null;
                const index = Number(key);
                return Number.isSafeInteger(index) && index < 0xffffffff ? index : null;
            }
            function createMap(element) {
                const target = Object.create(NamedNodeMap.prototype);
                const map = new Proxy(target, {
                    get(target, key, receiver) {
                        if (key === 'length') return records(element).length;
                        const index = indexFromKey(key);
                        if (index !== null) {
                            const current = records(element);
                            return index < current.length ? attrFromRecord(element, current[index]) : undefined;
                        }
                        return Reflect.get(target, key, receiver);
                    },
                    has(target, key) {
                        if (key === 'length') return true;
                        const index = indexFromKey(key);
                        if (index !== null) return index < records(element).length;
                        return Reflect.has(target, key);
                    },
                    ownKeys(target) {
                        return [...records(element).map((_, index) => String(index)), ...Reflect.ownKeys(target)];
                    },
                    getOwnPropertyDescriptor(target, key) {
                        const index = indexFromKey(key);
                        if (index !== null) {
                            const current = records(element);
                            if (index < current.length) {
                                return {value: attrFromRecord(element, current[index]), enumerable: true, configurable: true, writable: false};
                            }
                            return undefined;
                        }
                        return Reflect.getOwnPropertyDescriptor(target, key);
                    },
                    set(target, key, value, receiver) {
                        if (key === 'length' || indexFromKey(key) !== null) return false;
                        return Reflect.set(target, key, value, receiver);
                    },
                    deleteProperty(target, key) {
                        if (key === 'length' || indexFromKey(key) !== null) return false;
                        return Reflect.deleteProperty(target, key);
                    },
                });
                mapOwners.set(map, element);
                return map;
            }

            Object.defineProperty(Attr.prototype, 'name', {get() { return attrState(this).name; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'localName', {get() { return attrState(this).localName; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'namespaceURI', {get() { return attrState(this).namespaceURI; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'prefix', {get() { return attrState(this).prefix; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'ownerElement', {get() { return attrState(this).ownerElement; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'ownerDocument', {get() { return attrState(this).ownerDocument; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'specified', {get() { attrState(this); return true; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'value', {get() { const state = attrState(this); return state.ownerElement ? state.ownerElement.getAttributeNS(state.namespaceURI, state.localName) ?? state.value : state.value; }, set(value) { setAttrValue(this, value); }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'nodeType', {value: 2, enumerable: true});
            Object.defineProperty(Attr.prototype, 'nodeName', {get() { return attrState(this).name; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'nodeValue', {get() { return this.value; }, set(value) { this.value = value; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'textContent', {get() { return this.value; }, set(value) { this.value = value; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'parentNode', {get() { attrState(this); return null; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'parentElement', {get() { attrState(this); return null; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'firstChild', {get() { attrState(this); return null; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'lastChild', {get() { attrState(this); return null; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'nextSibling', {get() { attrState(this); return null; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'previousSibling', {get() { attrState(this); return null; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'childNodes', {get() { attrState(this); return emptyChildNodes; }, enumerable: true, configurable: true});
            Object.defineProperty(Attr.prototype, 'isConnected', {get() { attrState(this); return false; }, enumerable: true, configurable: true});
            Attr.prototype.hasChildNodes = function() { attrState(this); return false; };
            Attr.prototype.getRootNode = function() { attrState(this); return this; };
            Attr.prototype.contains = function(node) { attrState(this); return node === this; };
            Attr.prototype.cloneNode = function() {
                const state = attrState(this);
                return makeAttr(state, null, state.ownerDocument);
            };
            Attr.prototype.normalize = function() { attrState(this); };
            for (const name of ['appendChild', 'insertBefore', 'removeChild', 'replaceChild']) {
                Object.defineProperty(Attr.prototype, name, {
                    value() { attrState(this); throw new DOMException('Attributes cannot have children', 'HierarchyRequestError'); },
                    writable: true, configurable: true,
                });
            }

            NamedNodeMap.prototype.item = function(index) {
                const element = mapOwner(this);
                const number = Number(index);
                if (!Number.isFinite(number) || number < 0) return null;
                const position = Math.trunc(number) >>> 0;
                const current = records(element);
                return position < current.length ? attrFromRecord(element, current[position]) : null;
            };
            Object.defineProperty(NamedNodeMap.prototype, 'length', {
                get() { return records(mapOwner(this)).length; }, enumerable: true, configurable: true,
            });
            NamedNodeMap.prototype.getNamedItem = function(name) {
                const element = mapOwner(this);
                const record = findByName(element, name);
                return record ? attrFromRecord(element, record) : null;
            };
            NamedNodeMap.prototype.getNamedItemNS = function(namespaceURI, localName) {
                const element = mapOwner(this);
                const namespace = namespaceOrNull(namespaceURI);
                const record = records(element).find(item => item.namespaceURI === namespace && item.localName === String(localName));
                return record ? attrFromRecord(element, record) : null;
            };
            NamedNodeMap.prototype.setNamedItem = function(attr) { return setNamedItem(this, attr, false); };
            NamedNodeMap.prototype.setNamedItemNS = function(attr) { return setNamedItem(this, attr, true); };
            NamedNodeMap.prototype.removeNamedItem = function(name) { return removeNamedItem(this, name, false, null); };
            NamedNodeMap.prototype.removeNamedItemNS = function(namespaceURI, localName) {
                return removeNamedItem(this, localName, true, namespaceOrNull(namespaceURI));
            };

            Object.defineProperty(Element.prototype, 'attributes', {
                get() {
                    if (!this || this.nodeType !== 1 || typeof this.__getAttributeRecords !== 'function') throw new TypeError('Illegal invocation');
                    let map = mapForElement.get(this);
                    if (!map) { map = createMap(this); mapForElement.set(this, map); }
                    return map;
                }, enumerable: true, configurable: true,
            });
            const mapForElement = new WeakMap();
            Object.defineProperty(Element.prototype, 'hasAttributes', {
                value: function() {
                    if (!this || this.nodeType !== 1 || typeof this.__getAttributeRecords !== 'function') throw new TypeError('Illegal invocation');
                    return records(this).length !== 0;
                }, writable: true, configurable: true,
            });
            Element.prototype.getAttributeNS = function(namespaceURI, localName) {
                return this.__getAttributeNS(namespaceOrNull(namespaceURI), String(localName));
            };
            Element.prototype.hasAttributeNS = function(namespaceURI, localName) {
                return this.__hasAttributeNS(namespaceOrNull(namespaceURI), String(localName));
            };
            Element.prototype.removeAttributeNS = function(namespaceURI, localName) {
                return this.__removeAttributeNS(namespaceOrNull(namespaceURI), String(localName));
            };
            Element.prototype.setAttributeNS = function(namespaceURI, qualifiedName, value) {
                const namespace = namespaceOrNull(namespaceURI);
                const name = String(qualifiedName);
                if (!/^[A-Za-z_\u0080-\uFFFF][A-Za-z0-9_.\-\u0080-\uFFFF]*(:[A-Za-z_\u0080-\uFFFF][A-Za-z0-9_.\-\u0080-\uFFFF]*)?$/.test(name)) {
                    throw new DOMException('The qualified name is not a valid XML name', 'InvalidCharacterError');
                }
                const colon = name.indexOf(':');
                const prefix = colon < 0 ? null : name.slice(0, colon);
                const localName = colon < 0 ? name : name.slice(colon + 1);
                if ((prefix && namespace === null) || (prefix === 'xml' && namespace !== 'http://www.w3.org/XML/1998/namespace') ||
                    ((name === 'xmlns' || prefix === 'xmlns') && namespace !== 'http://www.w3.org/2000/xmlns/') ||
                    (namespace === 'http://www.w3.org/2000/xmlns/' && name !== 'xmlns' && prefix !== 'xmlns')) {
                    throw new DOMException('The prefix and namespace URI are inconsistent', 'NamespaceError');
                }
                return this.__setAttributeRecord(namespace, prefix, localName, String(value));
            };
            Element.prototype.getAttributeNode = function(name) { return this.attributes.getNamedItem(name); };
            Element.prototype.getAttributeNodeNS = function(namespaceURI, localName) { return this.attributes.getNamedItemNS(namespaceURI, localName); };
            Element.prototype.setAttributeNode = function(attr) { return this.attributes.setNamedItem(attr); };
            Element.prototype.setAttributeNodeNS = function(attr) { return this.attributes.setNamedItemNS(attr); };

            Object.defineProperty(Node, 'ATTRIBUTE_NODE', {value: 2, enumerable: true});
            Object.defineProperty(Node.prototype, 'ATTRIBUTE_NODE', {value: 2, enumerable: true});
            return function(element, namespaceURI, localName, value) {
                const cache = attrCaches.get(element);
                if (!cache) return;
                const current = value === null ? null : element.__getAttributeRecords().find(record =>
                    record.namespaceURI === namespaceURI && record.localName === localName);
                for (const attr of cache.values()) {
                    const state = attrState(attr);
                    if (state.ownerElement !== element || state.namespaceURI !== namespaceURI || state.localName !== localName) continue;
                    if (!current) state.ownerElement = null;
                    else {
                        state.name = current.name;
                        state.localName = current.localName;
                        state.namespaceURI = current.namespaceURI;
                        state.prefix = current.prefix;
                        state.value = current.value;
                    }
                }
            };
        })()
        "#,
    ))?;
    let callback = callback.as_object().ok_or_else(|| {
        JsNativeError::error().with_message("attribute mutation callback unavailable")
    })?;
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        state.borrow_mut().attribute_mutation_callback = Some(callback);
    }
    Ok(())
}
