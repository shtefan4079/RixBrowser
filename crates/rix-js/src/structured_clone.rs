
use boa_engine::{
    builtins::{
        array_buffer::{ArrayBuffer, SharedArrayBuffer},
        date::Date,
        error::Error,
        map::ordered_map::OrderedMap,
        regexp::RegExp,
        set::ordered_set::OrderedSet,
        typed_array::{TypedArray, TypedArrayKind},
    },
    js_string,
    native_function::NativeFunction,
    object::builtins::{JsArray, JsArrayBuffer, JsDataView, JsTypedArray},
    property::{PropertyDescriptor, PropertyKey},
    Context, JsBigInt, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[derive(Clone)]
struct CloneIntrinsics {
    exception: JsObject,
    exception_name: JsObject,
    exception_message: JsObject,
    transfer_sequence: JsObject,
    buffer_resizable: JsObject,
    mark_platform: JsObject,
    is_platform: JsObject,
    message_port_hooks: Rc<RefCell<Option<MessagePortTransferHooks>>>,
}

#[derive(Clone)]
struct MessagePortTransferHooks {
    is_message_port: JsObject,
    prepare_transfer: JsObject,
    validate_transfer: JsObject,
    commit_transfer: JsObject,
    abort_transfer: JsObject,
}

pub(crate) fn register(context: &mut Context) -> JsResult<()> {
    if context.get_data::<CloneIntrinsics>().is_some() {
        return Ok(());
    }
    let exception = context
        .global_object()
        .get(js_string!("DOMException"), context)?
        .as_object()
        .ok_or_else(invariant_violation)?;
    let prototype = exception
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(invariant_violation)?;
    let getter = |object: &JsObject, name: &str| {
        object
            .borrow()
            .properties()
            .get(&PropertyKey::from(JsString::from(name)))
            .and_then(|descriptor| descriptor.get().cloned())
            .and_then(|value| value.as_object())
            .ok_or_else(invariant_violation)
    };
    let exception_name = getter(&prototype, "name")?;
    let exception_message = getter(&prototype, "message")?;
    let buffer_resizable = getter(
        &context
            .intrinsics()
            .constructors()
            .array_buffer()
            .prototype(),
        "resizable",
    )?;
    let transfer_sequence = context.eval(boa_engine::Source::from_bytes(r#"
        (function() {
            const from = Array.from, ArrayConstructor = Array, apply = Reflect.apply;
            const iteratorSymbol = Symbol.iterator, TypeErrorConstructor = TypeError;
            const isObject = value => value !== null && (typeof value === 'object' || typeof value === 'function');
            return function transferSequence(value) {
                if (!isObject(value)) throw new TypeErrorConstructor('transfer must be an iterable object');
                const method = value[iteratorSymbol];
                if (typeof method !== 'function') throw new TypeErrorConstructor('transfer must be an iterable object');
                const iterator = apply(method, value, []);
                if (!isObject(iterator)) throw new TypeErrorConstructor('transfer iterator must be an object');
                const iterable = {[iteratorSymbol]: function() { return iterator; }};
                return apply(from, ArrayConstructor, [iterable, function(entry) {
                    if (!isObject(entry)) throw new TypeErrorConstructor('Each transfer entry must be an object');
                    return entry;
                }]);
            };
        })()
    "#))?.as_object().ok_or_else(invariant_violation)?;
    let platform = context.eval(boa_engine::Source::from_bytes(r#"
        (function() {
            const identities = new WeakSet(), add = WeakSet.prototype.add, has = WeakSet.prototype.has;
            const apply = Reflect.apply;
            return [
                function markPlatform(value) { apply(add, identities, [value]); return value; },
                function isPlatform(value) { return apply(has, identities, [value]); },
            ];
        })()
    "#))?.as_object().ok_or_else(invariant_violation)?;
    let mark_platform = platform
        .get(0u32, context)?
        .as_object()
        .ok_or_else(invariant_violation)?;
    let is_platform = platform
        .get(1u32, context)?
        .as_object()
        .ok_or_else(invariant_violation)?;
    context.insert_data(CloneIntrinsics {
        exception,
        exception_name,
        exception_message,
        transfer_sequence,
        buffer_resizable,
        mark_platform,
        is_platform,
        message_port_hooks: Rc::new(RefCell::new(None)),
    });
    context.register_global_builtin_callable(
        js_string!("structuredClone"),
        1,
        NativeFunction::from_fn_ptr(structured_clone),
    )?;
    context.register_global_builtin_callable(
        js_string!("__rixSerializeWindowMessage"),
        2,
        NativeFunction::from_fn_ptr(serialize_window_message),
    )?;
    Ok(())
}

pub(crate) fn register_message_port_transfer_hooks(
    context: &mut Context,
    hooks: &JsObject,
) -> JsResult<()> {
    let get_hook = |name: &str, context: &mut Context| -> JsResult<JsObject> {
        hooks
            .get(JsString::from(name), context)?
            .as_object()
            .ok_or_else(invariant_violation)
    };
    let hooks = MessagePortTransferHooks {
        is_message_port: get_hook("isMessagePort", context)?,
        prepare_transfer: get_hook("prepareTransfer", context)?,
        validate_transfer: get_hook("validateTransfer", context)?,
        commit_transfer: get_hook("commitTransfer", context)?,
        abort_transfer: get_hook("abortTransfer", context)?,
    };
    let intrinsics = context
        .get_data::<CloneIntrinsics>()
        .ok_or_else(invariant_violation)?;
    *intrinsics.message_port_hooks.borrow_mut() = Some(hooks);
    Ok(())
}

pub(crate) fn register_platform_objects(context: &mut Context, script: &str) -> JsResult<()> {
    register_platform_objects_with_result(context, script).map(|_| ())
}

pub(crate) fn register_platform_objects_with_result(
    context: &mut Context,
    script: &str,
) -> JsResult<JsValue> {
    let marker = context
        .get_data::<CloneIntrinsics>()
        .ok_or_else(invariant_violation)?
        .mark_platform
        .clone();
    let function = context
        .eval(boa_engine::Source::from_bytes(script.as_bytes()))?
        .as_object()
        .ok_or_else(invariant_violation)?;
    function.call(&JsValue::undefined(), &[marker.into()], context)
}

pub(crate) fn mark_platform_object(context: &mut Context, object: &JsObject) -> JsResult<()> {
    let marker = context
        .get_data::<CloneIntrinsics>()
        .ok_or_else(invariant_violation)?
        .mark_platform
        .clone();
    marker.call(&JsValue::undefined(), &[object.clone().into()], context)?;
    Ok(())
}

fn invariant_violation() -> JsError {
    JsNativeError::typ()
        .with_message("structuredClone internal invariant violated")
        .into()
}

fn data_clone_error(message: &str, context: &mut Context) -> JsError {
    let Some(constructor) = context
        .get_data::<CloneIntrinsics>()
        .map(|intrinsics| intrinsics.exception.clone())
    else {
        return invariant_violation();
    };
    match constructor.construct(
        &[
            JsString::from(message).into(),
            js_string!("DataCloneError").into(),
        ],
        None,
        context,
    ) {
        Ok(exception) => JsError::from_opaque(exception.into()),
        Err(error) => error,
    }
}

fn transfer_value(options: &JsValue, context: &mut Context) -> JsResult<JsValue> {
    if options.is_null_or_undefined() {
        return Ok(JsValue::undefined());
    }
    let options = options.as_object().ok_or_else(|| {
        JsNativeError::typ().with_message("structuredClone options must be a dictionary")
    })?;
    options.get(js_string!("transfer"), context)
}

fn validate_transfer(buffer: &JsArrayBuffer, context: &mut Context) -> JsResult<()> {
    let native = buffer.borrow().data().clone();
    if native.data().is_none() {
        return Err(data_clone_error(
            "A detached ArrayBuffer cannot be transferred",
            context,
        ));
    }
    let mut disposable = native;
    disposable
        .detach(&JsValue::undefined())
        .map_err(|_| data_clone_error("This ArrayBuffer cannot be detached", context))?;
    Ok(())
}

enum Frame {
    Properties {
        source: JsObject,
        target: JsObject,
        keys: Vec<PropertyKey>,
        next: usize,
    },
    Map {
        target: JsObject,
        entries: Vec<(JsValue, JsValue)>,
        next: usize,
    },
    MapValue {
        target: JsObject,
        key: JsValue,
        value: JsValue,
    },
    Set {
        target: JsObject,
        entries: Vec<JsValue>,
        next: usize,
    },
}

struct Cloner {
    memory: HashMap<JsObject, JsObject>,
    stack: Vec<Frame>,
    dom_objects: HashSet<JsObject>,
    transferred_message_ports: HashMap<JsObject, JsObject>,
}

impl Cloner {
    fn new(context: &Context) -> Self {
        let mut dom_objects = HashSet::from([context.global_object()]);
        if let Some(state) = context.get_data::<Rc<RefCell<crate::dom_bridge::DomBridgeState>>>() {
            dom_objects.extend(state.borrow().element_wrappers.values().cloned());
        }
        Self {
            memory: HashMap::new(),
            stack: Vec::new(),
            dom_objects,
            transferred_message_ports: HashMap::new(),
        }
    }

    fn clone_value(&mut self, value: &JsValue, context: &mut Context) -> JsResult<JsValue> {
        if value.is_symbol() {
            return Err(data_clone_error("A Symbol value cannot be cloned", context));
        }
        let Some(source) = value.as_object() else {
            return Ok(value.clone());
        };
        if let Some(target) = self.memory.get(&source) {
            return Ok(target.clone().into());
        }
        if let Some(target) = self.transferred_message_ports.get(&source) {
            self.memory.insert(source, target.clone());
            return Ok(target.clone().into());
        }
        let is_platform = context
            .get_data::<CloneIntrinsics>()
            .ok_or_else(invariant_violation)?
            .is_platform
            .clone();
        if is_platform
            .call(value, std::slice::from_ref(value), context)?
            .to_boolean()
        {
            return Err(data_clone_error(
                "This platform object cannot be cloned",
                context,
            ));
        }
        let is_dom = self.dom_objects.contains(&source)
            || context
                .get_data::<Rc<RefCell<crate::dom_bridge::DomBridgeState>>>()
                .is_some_and(|state| state.borrow().wrapper_nodes.contains_key(&source));
        if source.is_callable() || is_dom {
            return Err(data_clone_error(
                "Functions and DOM objects cannot be cloned",
                context,
            ));
        }
        let target = if let Some(data) = source.downcast_ref::<Date>() {
            JsObject::from_proto_and_data(
                context.intrinsics().constructors().date().prototype(),
                *data,
            )
        } else if let Some(data) = source.downcast_ref::<RegExp>() {
            let target = JsObject::from_proto_and_data(
                context.intrinsics().constructors().regexp().prototype(),
                data.clone(),
            );
            target.define_property_or_throw(
                js_string!("lastIndex"),
                PropertyDescriptor::builder()
                    .value(0)
                    .writable(true)
                    .enumerable(false)
                    .configurable(false),
                context,
            )?;
            target
        } else if let Some(data) = source.downcast_ref::<ArrayBuffer>() {
            if data.data().is_none() {
                return Err(data_clone_error(
                    "A detached ArrayBuffer cannot be cloned",
                    context,
                ));
            }
            JsObject::from_proto_and_data(
                context
                    .intrinsics()
                    .constructors()
                    .array_buffer()
                    .prototype(),
                data.clone(),
            )
        } else if source.is::<SharedArrayBuffer>() {
            return Err(data_clone_error(
                "SharedArrayBuffer requires cross-origin isolation",
                context,
            ));
        } else if source.is::<TypedArray>() {
            let array = JsTypedArray::from_object(source.clone())?;
            let buffer = array.buffer(context)?;
            self.reject_resizable_view(&buffer, context)?;
            let target_buffer = self.clone_value(&buffer, context)?;
            let offset = array.byte_offset(context)?;
            let length = array.length(context)?;
            let constructor =
                typed_array_constructor(array.kind().ok_or_else(invariant_violation)?, context);
            constructor.construct(
                &[target_buffer, offset.into(), length.into()],
                None,
                context,
            )?
        } else if let Ok(view) = JsDataView::from_object(source.clone()) {
            let buffer = view.buffer(context)?;
            self.reject_resizable_view(&buffer, context)?;
            let offset = view.byte_offset(context).map_err(|_| {
                data_clone_error("An out-of-bounds DataView cannot be cloned", context)
            })?;
            let length = view.byte_length(context).map_err(|_| {
                data_clone_error("An out-of-bounds DataView cannot be cloned", context)
            })?;
            let target_buffer = self.clone_value(&buffer, context)?;
            context
                .intrinsics()
                .constructors()
                .data_view()
                .constructor()
                .construct(
                    &[target_buffer, offset.into(), length.into()],
                    None,
                    context,
                )?
        } else if let Some(data) = source.downcast_ref::<OrderedMap<JsValue>>() {
            let entries = data
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            let target = JsObject::from_proto_and_data(
                context.intrinsics().constructors().map().prototype(),
                OrderedMap::<JsValue>::new(),
            );
            self.stack.push(Frame::Map {
                target: target.clone(),
                entries,
                next: 0,
            });
            target
        } else if let Some(data) = source.downcast_ref::<OrderedSet>() {
            let entries = data.iter().cloned().collect();
            let target = JsObject::from_proto_and_data(
                context.intrinsics().constructors().set().prototype(),
                OrderedSet::new(),
            );
            self.stack.push(Frame::Set {
                target: target.clone(),
                entries,
                next: 0,
            });
            target
        } else if let Some(data) = source.downcast_ref::<bool>() {
            JsObject::from_proto_and_data(
                context.intrinsics().constructors().boolean().prototype(),
                *data,
            )
        } else if let Some(data) = source.downcast_ref::<f64>() {
            JsObject::from_proto_and_data(
                context.intrinsics().constructors().number().prototype(),
                *data,
            )
        } else if let Some(data) = source.downcast_ref::<JsBigInt>() {
            JsObject::from_proto_and_data(
                context.intrinsics().constructors().bigint().prototype(),
                data.clone(),
            )
        } else if let Some(data) = source.downcast_ref::<JsString>() {
            let target = JsObject::from_proto_and_data(
                context.intrinsics().constructors().string().prototype(),
                data.clone(),
            );
            target.define_property_or_throw(
                js_string!("length"),
                PropertyDescriptor::builder()
                    .value(data.len())
                    .writable(false)
                    .enumerable(false)
                    .configurable(false),
                context,
            )?;
            target
        } else if source.is::<Error>() {
            self.clone_error(&source, context)?
        } else if source.is_array() || source.is_ordinary() {
            let target = if source.is_array() {
                let target: JsObject = JsArray::new(context)?.into();
                let length = source
                    .borrow()
                    .properties()
                    .get(&PropertyKey::from(js_string!("length")))
                    .ok_or_else(invariant_violation)?
                    .value()
                    .ok_or_else(invariant_violation)?
                    .clone();
                target.set(js_string!("length"), length, true, context)?;
                target
            } else {
                JsObject::with_object_proto(context.intrinsics())
            };
            let keys = source
                .own_property_keys(context)?
                .into_iter()
                .filter(|key| {
                    !matches!(key, PropertyKey::Symbol(_))
                        && source
                            .borrow()
                            .properties()
                            .get(key)
                            .is_some_and(|descriptor| descriptor.enumerable() == Some(true))
                })
                .collect();
            self.stack.push(Frame::Properties {
                source: source.clone(),
                target: target.clone(),
                keys,
                next: 0,
            });
            target
        } else {
            return Err(data_clone_error(
                "This object has non-serializable internal data",
                context,
            ));
        };
        self.memory.insert(source, target.clone());
        Ok(target.into())
    }

    fn reject_resizable_view(&self, buffer: &JsValue, context: &mut Context) -> JsResult<()> {
        let getter = context
            .get_data::<CloneIntrinsics>()
            .ok_or_else(invariant_violation)?
            .buffer_resizable
            .clone();
        if buffer
            .as_object()
            .is_some_and(|object| object.is::<ArrayBuffer>())
            && getter.call(buffer, &[], context)?.to_boolean()
        {
            return Err(data_clone_error(
                "Cloning views of resizable ArrayBuffers is not implemented",
                context,
            ));
        }
        Ok(())
    }

    fn clone_error(&self, source: &JsObject, context: &mut Context) -> JsResult<JsObject> {
        let intrinsics = context
            .get_data::<CloneIntrinsics>()
            .ok_or_else(invariant_violation)?
            .clone();
        if let Ok(name) = intrinsics
            .exception_name
            .call(&source.clone().into(), &[], context)
        {
            let message =
                intrinsics
                    .exception_message
                    .call(&source.clone().into(), &[], context)?;
            return intrinsics
                .exception
                .construct(&[message, name], None, context);
        }
        let name = source.get(js_string!("name"), context)?;
        let constructors = context.intrinsics().constructors();
        let constructor = match name
            .as_string()
            .map(|name| name.to_std_string_escaped())
            .as_deref()
        {
            Some("EvalError") => constructors.eval_error().constructor(),
            Some("RangeError") => constructors.range_error().constructor(),
            Some("ReferenceError") => constructors.reference_error().constructor(),
            Some("SyntaxError") => constructors.syntax_error().constructor(),
            Some("TypeError") => constructors.type_error().constructor(),
            Some("URIError") => constructors.uri_error().constructor(),
            _ => constructors.error().constructor(),
        };
        let message = source
            .borrow()
            .properties()
            .get(&PropertyKey::from(js_string!("message")))
            .and_then(|descriptor| descriptor.value().cloned());
        let arguments = match message {
            Some(message) => vec![message.to_string(context)?.into()],
            None => Vec::new(),
        };
        constructor.construct(&arguments, None, context)
    }

    fn drain(&mut self, context: &mut Context) -> JsResult<()> {
        while let Some(frame) = self.stack.pop() {
            match frame {
                Frame::Properties {
                    source,
                    target,
                    keys,
                    mut next,
                } => {
                    if next == keys.len() {
                        continue;
                    }
                    let key = keys[next].clone();
                    next += 1;
                    self.stack.push(Frame::Properties {
                        source: source.clone(),
                        target: target.clone(),
                        keys,
                        next,
                    });
                    if source.has_own_property(key.clone(), context)? {
                        let value = source.get(key.clone(), context)?;
                        let cloned = self.clone_value(&value, context)?;
                        target.create_data_property_or_throw(key, cloned, context)?;
                    }
                }
                Frame::Map {
                    target,
                    entries,
                    next,
                } => {
                    if next == entries.len() {
                        continue;
                    }
                    let (key, value) = entries[next].clone();
                    self.stack.push(Frame::Map {
                        target: target.clone(),
                        entries,
                        next: next + 1,
                    });
                    let stack_length = self.stack.len();
                    let cloned_key = self.clone_value(&key, context)?;
                    let children = self.stack.split_off(stack_length);
                    self.stack.push(Frame::MapValue {
                        target,
                        key: cloned_key,
                        value,
                    });
                    self.stack.extend(children);
                }
                Frame::MapValue { target, key, value } => {
                    let cloned_value = self.clone_value(&value, context)?;
                    target
                        .downcast_mut::<OrderedMap<JsValue>>()
                        .ok_or_else(invariant_violation)?
                        .insert(key, cloned_value);
                }
                Frame::Set {
                    target,
                    entries,
                    next,
                } => {
                    if next == entries.len() {
                        continue;
                    }
                    let value = entries[next].clone();
                    self.stack.push(Frame::Set {
                        target: target.clone(),
                        entries,
                        next: next + 1,
                    });
                    let cloned = self.clone_value(&value, context)?;
                    target
                        .downcast_mut::<OrderedSet>()
                        .ok_or_else(invariant_violation)?
                        .add(cloned);
                }
            }
        }
        Ok(())
    }
}

fn typed_array_constructor(kind: TypedArrayKind, context: &Context) -> JsObject {
    let constructors = context.intrinsics().constructors();
    match kind {
        TypedArrayKind::Int8 => constructors.typed_int8_array(),
        TypedArrayKind::Uint8 => constructors.typed_uint8_array(),
        TypedArrayKind::Uint8Clamped => constructors.typed_uint8clamped_array(),
        TypedArrayKind::Int16 => constructors.typed_int16_array(),
        TypedArrayKind::Uint16 => constructors.typed_uint16_array(),
        TypedArrayKind::Int32 => constructors.typed_int32_array(),
        TypedArrayKind::Uint32 => constructors.typed_uint32_array(),
        TypedArrayKind::BigInt64 => constructors.typed_bigint64_array(),
        TypedArrayKind::BigUint64 => constructors.typed_biguint64_array(),
        TypedArrayKind::Float16 => constructors.typed_float16_array(),
        TypedArrayKind::Float32 => constructors.typed_float32_array(),
        TypedArrayKind::Float64 => constructors.typed_float64_array(),
    }
    .constructor()
}

pub(crate) fn clone_serializable_value(
    value: &JsValue,
    context: &mut Context,
) -> JsResult<JsValue> {
    let mut cloner = Cloner::new(context);
    let cloned = cloner.clone_value(value, context)?;
    cloner.drain(context)?;
    Ok(cloned)
}

fn structured_clone(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(value) = args.first() else {
        return Err(JsNativeError::typ()
            .with_message("structuredClone requires a value")
            .into());
    };
    let default_options = JsValue::undefined();
    let options = args.get(1).unwrap_or(&default_options);
    let transfer = transfer_value(options, context)?;
    serialize_with_transfer(value, &transfer, None, context, |cloned, _, _| Ok(cloned))
}

struct MessageTransferList {
    buffers: Vec<JsArrayBuffer>,
    ports: Vec<(JsObject, JsObject)>,
    port_hooks: Option<MessagePortTransferHooks>,
}

fn message_transfer_list(
    value: &JsValue,
    sender_port: Option<&JsObject>,
    context: &mut Context,
) -> JsResult<MessageTransferList> {
    if value.is_undefined() {
        return Ok(MessageTransferList {
            buffers: Vec::new(),
            ports: Vec::new(),
            port_hooks: None,
        });
    }
    let intrinsics = context
        .get_data::<CloneIntrinsics>()
        .ok_or_else(invariant_violation)?
        .clone();
    let entries = intrinsics
        .transfer_sequence
        .call(&JsValue::undefined(), std::slice::from_ref(value), context)?
        .as_object()
        .ok_or_else(invariant_violation)?;
    let entries = JsArray::from_object(entries)?;
    let length = entries.length(context)?;
    let port_hooks = intrinsics.message_port_hooks.borrow().clone();
    let mut seen = HashSet::new();
    let mut buffers = Vec::new();
    let mut port_sources = Vec::new();
    for index in 0..length {
        let entry = entries
            .get(index, context)?
            .as_object()
            .ok_or_else(|| JsNativeError::typ().with_message("Transfer entries must be objects"))?;
        if !seen.insert(entry.clone()) {
            return Err(data_clone_error(
                "The transfer list contains the same object twice",
                context,
            ));
        }
        if entry.is::<ArrayBuffer>() {
            let buffer = JsArrayBuffer::from_object(entry)?;
            validate_transfer(&buffer, context)?;
            buffers.push(buffer);
            continue;
        }
        let is_port = if let Some(hooks) = &port_hooks {
            hooks
                .is_message_port
                .call(&JsValue::undefined(), &[entry.clone().into()], context)?
                .to_boolean()
        } else {
            false
        };
        if !is_port {
            return Err(data_clone_error(
                "This object is not a supported transferable",
                context,
            ));
        }
        port_sources.push(entry);
    }

    if sender_port.is_some_and(|sender| port_sources.iter().any(|source| source == sender)) {
        return Err(data_clone_error(
            "A MessagePort cannot transfer itself",
            context,
        ));
    }

    let mut ports = Vec::with_capacity(port_sources.len());
    if let Some(hooks) = &port_hooks {
        for source in port_sources {
            let target = match hooks.prepare_transfer.call(
                &JsValue::undefined(),
                &[source.clone().into()],
                context,
            ) {
                Ok(target) => match target.as_object() {
                    Some(target) => target,
                    None => {
                        let error = invariant_violation();
                        abort_message_ports(&ports, hooks, context)?;
                        return Err(error);
                    }
                },
                Err(error) => {
                    abort_message_ports(&ports, hooks, context)?;
                    return Err(error);
                }
            };
            ports.push((source, target));
        }
    } else if !port_sources.is_empty() {
        return Err(data_clone_error(
            "MessagePort transfer is unavailable in this realm",
            context,
        ));
    }

    Ok(MessageTransferList {
        buffers,
        ports,
        port_hooks,
    })
}

fn abort_message_ports(
    ports: &[(JsObject, JsObject)],
    hooks: &MessagePortTransferHooks,
    context: &mut Context,
) -> JsResult<()> {
    for (source, _) in ports {
        hooks
            .abort_transfer
            .call(&JsValue::undefined(), &[source.clone().into()], context)?;
    }
    Ok(())
}

fn serialize_with_transfer<F>(
    message: &JsValue,
    transfer: &JsValue,
    sender_port: Option<&JsObject>,
    context: &mut Context,
    build_result: F,
) -> JsResult<JsValue>
where
    F: FnOnce(JsValue, &[JsObject], &mut Context) -> JsResult<JsValue>,
{
    let transfers = message_transfer_list(transfer, sender_port, context)?;
    let mut cloner = Cloner::new(context);
    cloner.transferred_message_ports = transfers
        .ports
        .iter()
        .map(|(source, target)| (source.clone(), target.clone()))
        .collect();

    let serialization = (|| {
        for buffer in &transfers.buffers {
            let object: JsObject = buffer.clone().into();
            cloner.clone_value(&object.into(), context)?;
        }
        let cloned = cloner.clone_value(message, context)?;
        cloner.drain(context)?;

        let mut final_buffers = Vec::with_capacity(transfers.buffers.len());
        for buffer in &transfers.buffers {
            validate_transfer(buffer, context)?;
            final_buffers.push(buffer.borrow().data().clone());
        }
        if let Some(hooks) = &transfers.port_hooks {
            for (source, target) in &transfers.ports {
                hooks.validate_transfer.call(
                    &JsValue::undefined(),
                    &[source.clone().into(), target.clone().into()],
                    context,
                )?;
            }
        }
        for (buffer, native) in transfers.buffers.iter().zip(final_buffers) {
            let source = JsObject::from(buffer.clone());
            let target = cloner.memory.get(&source).ok_or_else(invariant_violation)?;
            *target
                .downcast_mut::<ArrayBuffer>()
                .ok_or_else(invariant_violation)? = native;
        }

        let transferred_ports = transfers
            .ports
            .iter()
            .map(|(_, target)| target.clone())
            .collect::<Vec<_>>();
        let result = build_result(cloned, &transferred_ports, context)?;

        for buffer in &transfers.buffers {
            buffer.detach(&JsValue::undefined())?;
        }
        if let Some(hooks) = &transfers.port_hooks {
            for (source, target) in &transfers.ports {
                hooks.commit_transfer.call(
                    &JsValue::undefined(),
                    &[source.clone().into(), target.clone().into()],
                    context,
                )?;
            }
        }
        Ok(result)
    })();

    if serialization.is_err() {
        if let Some(hooks) = &transfers.port_hooks {
            abort_message_ports(&transfers.ports, hooks, context)?;
        }
    }
    serialization
}

fn serialize_window_message(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(message) = args.first() else {
        return Err(JsNativeError::typ()
            .with_message("Window.postMessage requires a message")
            .into());
    };
    let sender_port = args.get(2).and_then(JsValue::as_object);
    serialize_with_transfer(
        message,
        args.get(1).unwrap_or(&JsValue::undefined()),
        sender_port.as_ref(),
        context,
        |cloned, transferred_ports, context| {
            let ports: JsObject = JsArray::new(context)?.into();
            for (index, port) in transferred_ports.iter().enumerate() {
                ports.create_data_property_or_throw(
                    PropertyKey::from(index as u32),
                    port.clone(),
                    context,
                )?;
            }
            let result = JsObject::with_object_proto(context.intrinsics());
            result.create_data_property_or_throw(js_string!("data"), cloned, context)?;
            result.create_data_property_or_throw(js_string!("ports"), ports, context)?;
            Ok(result.into())
        },
    )
}
