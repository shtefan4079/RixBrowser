use super::*;

pub(super) fn register_timer_functions(context: &mut Context) -> Result<(), JsError> {
    context.register_global_callable(
        js_string!("setTimeout"),
        2,
        NativeFunction::from_fn_ptr(set_timeout_fn),
    )?;

    context.register_global_callable(
        js_string!("clearTimeout"),
        1,
        NativeFunction::from_fn_ptr(clear_timeout_fn),
    )?;

    context.register_global_callable(
        js_string!("setInterval"),
        2,
        NativeFunction::from_fn_ptr(set_interval_fn),
    )?;

    context.register_global_callable(
        js_string!("clearInterval"),
        1,
        NativeFunction::from_fn_ptr(clear_interval_fn),
    )?;

    context.register_global_callable(
        js_string!("queueMicrotask"),
        1,
        NativeFunction::from_fn_ptr(queue_microtask_fn),
    )?;

    Ok(())
}

pub(super) fn parse_timer_callback(
    args: &[JsValue],
    context: &mut Context,
    fn_name: &str,
) -> JsResult<(TimerCallback, Vec<JsValue>)> {
    let first = args.get_or_undefined(0);
    let extra: Vec<JsValue> = if args.len() > 2 {
        args[2..].to_vec()
    } else {
        Vec::new()
    };
    if let Some(callable) = first.as_callable() {
        return Ok((TimerCallback::Function(callable.clone()), extra));
    }
    if first.is_string() {
        let code = first
            .to_string(context)
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();
        return Ok((TimerCallback::Code(code), extra));
    }
    Err(JsNativeError::typ()
        .with_message(format!("{fn_name}: callback must be a function or string"))
        .into())
}

pub(super) fn set_timeout_fn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (callback, extra) = parse_timer_callback(args, context, "setTimeout")?;

    let delay_ms = args
        .get_or_undefined(1)
        .to_number(context)
        .unwrap_or(0.0)
        .max(0.0) as u64;

    let state = context.get_data::<Rc<RefCell<WebApiState>>>().cloned();
    if let Some(state) = state {
        let id = state.borrow_mut().schedule_timer(
            callback,
            extra,
            Duration::from_millis(delay_ms),
            None,
        );
        return Ok(JsValue::new(id));
    }
    Ok(JsValue::new(0))
}

pub(super) fn clear_timeout_fn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = args.get_or_undefined(0).to_number(context).unwrap_or(0.0) as u32;
    let state = context.get_data::<Rc<RefCell<WebApiState>>>().cloned();
    if let Some(state) = state {
        state.borrow_mut().cancel_timer(id);
    }
    Ok(JsValue::undefined())
}

pub(super) fn set_interval_fn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (callback, extra) = parse_timer_callback(args, context, "setInterval")?;

    let delay_ms = args
        .get_or_undefined(1)
        .to_number(context)
        .unwrap_or(0.0)
        .max(1.0) as u64;

    let state = context.get_data::<Rc<RefCell<WebApiState>>>().cloned();
    if let Some(state) = state {
        let dur = Duration::from_millis(delay_ms);
        let id = state
            .borrow_mut()
            .schedule_timer(callback, extra, dur, Some(dur));
        return Ok(JsValue::new(id));
    }
    Ok(JsValue::new(0))
}

pub(super) fn clear_interval_fn(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    clear_timeout_fn(_this, args, context)
}

pub(super) fn queue_microtask_fn(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let callback = args.get_or_undefined(0).as_callable().ok_or_else(|| {
        JsNativeError::typ().with_message("queueMicrotask: callback must be a function")
    })?;
    let extra: Vec<JsValue> = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        Vec::new()
    };
    let callback = callback.clone();
    context.enqueue_job(
        PromiseJob::new(move |context| {
            let this = context.global_object().into();
            callback.call(&this, &extra, context)
        })
        .into(),
    );
    Ok(JsValue::undefined())
}
