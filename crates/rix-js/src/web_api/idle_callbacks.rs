use super::*;

const IDLE_DEADLINE_FACTORY: &str = r#"
(function() {
    const slots = new WeakMap();
    const weakMapGet = Function.prototype.call.bind(WeakMap.prototype.get);
    const weakMapSet = Function.prototype.call.bind(WeakMap.prototype.set);
    const createObject = Object.create;
    const max = Math.max;
    const performanceNow = globalThis.performance.now.bind(globalThis.performance);

    function IdleDeadline() {
        throw new TypeError('Illegal constructor: IdleDeadline');
    }
    function getSlots(receiver) {
        const state = weakMapGet(slots, receiver);
        if (state === undefined) throw new TypeError('Illegal IdleDeadline receiver');
        return state;
    }

    const timeRemaining = { timeRemaining() {
        return max(0, getSlots(this).deadline - performanceNow());
    } }.timeRemaining;
    Object.defineProperties(IdleDeadline.prototype, {
        constructor: {value: IdleDeadline, writable: true, configurable: true},
        timeRemaining: {value: timeRemaining, writable: true, enumerable: true, configurable: true},
        didTimeout: {get: function didTimeout() {
            return getSlots(this).didTimeout;
        }, enumerable: true, configurable: true},
        [Symbol.toStringTag]: {value: 'IdleDeadline', configurable: true}
    });
    Object.defineProperty(globalThis, 'IdleDeadline', {
        value: IdleDeadline, writable: true, configurable: true
    });

    return function createIdleDeadline(deadline, didTimeout) {
        const object = createObject(IdleDeadline.prototype);
        weakMapSet(slots, object, {deadline, didTimeout});
        return object;
    };
})()
"#;

#[derive(Clone)]
pub(super) struct IdleDeadlineFactory(JsObject);

pub(super) fn register_idle_callback_api(context: &mut Context) -> Result<(), JsError> {
    register_window_operation(
        context,
        "requestIdleCallback",
        1,
        NativeFunction::from_fn_ptr(request_idle_callback),
    )?;
    register_window_operation(
        context,
        "cancelIdleCallback",
        1,
        NativeFunction::from_fn_ptr(cancel_idle_callback),
    )?;

    let factory = context.eval(boa_engine::Source::from_bytes(IDLE_DEADLINE_FACTORY))?;
    let factory = factory.as_object().ok_or_else(|| {
        JsNativeError::error().with_message("IdleDeadline factory did not return a function")
    })?;
    context.insert_data(IdleDeadlineFactory(factory));
    Ok(())
}

fn register_window_operation(
    context: &mut Context,
    name: &str,
    length: usize,
    body: NativeFunction,
) -> Result<(), JsError> {
    let window = context
        .global_object()
        .get(js_string!("Window"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Window constructor is unavailable"))?;
    let prototype = window
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Window prototype is unavailable"))?;
    let function = FunctionObjectBuilder::new(context.realm(), body)
        .name(JsString::from(name))
        .length(length)
        .constructor(false)
        .build();
    prototype.define_property_or_throw(
        JsString::from(name),
        PropertyDescriptor::builder()
            .value(function)
            .writable(true)
            .enumerable(true)
            .configurable(true),
        context,
    )?;
    Ok(())
}

fn request_idle_callback(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_window_receiver(this, context)?;
    let callback = args.get_or_undefined(0).as_callable().ok_or_else(|| {
        JsNativeError::typ().with_message("requestIdleCallback: callback must be callable")
    })?;

    let options = args.get_or_undefined(1);
    let timeout = if options.is_undefined() || options.is_null() {
        None
    } else {
        let options = options.to_object(context)?;
        let timeout = options.get(js_string!("timeout"), context)?;
        if timeout.is_undefined() {
            None
        } else {
            let timeout = to_unsigned_long(&timeout, context)?;
            (timeout > 0).then(|| Duration::from_millis(u64::from(timeout)))
        }
    };

    let state = context
        .get_data::<Rc<RefCell<WebApiState>>>()
        .cloned()
        .ok_or_else(|| {
            JsNativeError::error().with_message("requestIdleCallback: event loop is unavailable")
        })?;
    let id = state.borrow_mut().schedule_idle_callback(callback, timeout);
    Ok(JsValue::new(id))
}

fn cancel_idle_callback(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_window_receiver(this, context)?;
    let handle = args.get(0).ok_or_else(|| {
        JsNativeError::typ().with_message("cancelIdleCallback: handle is required")
    })?;
    let handle = to_unsigned_long(handle, context)?;
    if let Some(state) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        state.borrow_mut().cancel_idle_callback(handle);
    }
    Ok(JsValue::undefined())
}

fn require_window_receiver(this: &JsValue, context: &Context) -> JsResult<()> {
    if this.is_undefined()
        || this
            .as_object()
            .is_some_and(|receiver| receiver == context.global_object())
    {
        Ok(())
    } else {
        Err(JsNativeError::typ()
            .with_message("Illegal Window idle callback method receiver")
            .into())
    }
}

fn to_unsigned_long(value: &JsValue, context: &mut Context) -> JsResult<u32> {
    let number = value.to_number(context)?;
    if !number.is_finite() || number == 0.0 {
        return Ok(0);
    }
    Ok(number.trunc().rem_euclid(4_294_967_296.0) as u32)
}

pub(super) fn invoke_idle_callback(
    callback: &IdleCallback,
    deadline: Instant,
    did_timeout: bool,
    context: &mut Context,
) -> JsResult<JsValue> {
    let factory = context
        .get_data::<IdleDeadlineFactory>()
        .map(|factory| factory.0.clone())
        .ok_or_else(|| {
            JsNativeError::error().with_message("IdleDeadline factory is unavailable")
        })?;
    let now = Instant::now();
    let remaining_ms = if did_timeout {
        0.0
    } else {
        deadline.saturating_duration_since(now).as_secs_f64() * 1000.0
    };
    let performance_now = context
        .get_data::<PerformanceClock>()
        .map(PerformanceClock::elapsed_ms)
        .unwrap_or(0.0);
    let deadline_value = JsValue::new(performance_now + remaining_ms);
    let deadline_object = factory.call(
        &JsValue::undefined(),
        &[deadline_value, JsValue::new(did_timeout)],
        context,
    )?;
    let deadline_object = deadline_object.as_object().ok_or_else(|| {
        JsNativeError::error().with_message("IdleDeadline factory returned a non-object")
    })?;
    let this = context.global_object().into();
    callback
        .callback
        .call(&this, &[deadline_object.into()], context)
}
