use super::*;

pub(super) fn register_animation_frame_api(context: &mut Context) -> Result<(), JsError> {
    register_window_operation(
        context,
        "requestAnimationFrame",
        1,
        NativeFunction::from_fn_ptr(request_animation_frame),
    )?;
    register_window_operation(
        context,
        "cancelAnimationFrame",
        1,
        NativeFunction::from_fn_ptr(cancel_animation_frame),
    )?;
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

fn request_animation_frame(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_window_receiver(this, context)?;
    let callback = args.get_or_undefined(0).as_callable().ok_or_else(|| {
        JsNativeError::typ().with_message("requestAnimationFrame requires a callback")
    })?;
    let state = context
        .get_data::<Rc<RefCell<WebApiState>>>()
        .cloned()
        .ok_or_else(|| {
            JsNativeError::error().with_message("requestAnimationFrame: event loop is unavailable")
        })?;
    let id = state.borrow_mut().schedule_animation_frame(callback);
    Ok(JsValue::new(id))
}

fn cancel_animation_frame(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_window_receiver(this, context)?;
    let handle = to_unsigned_long(args.get_or_undefined(0), context)?;
    if let Some(state) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        state.borrow_mut().cancel_animation_frame(handle);
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
            .with_message("Illegal Window animation frame method receiver")
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
