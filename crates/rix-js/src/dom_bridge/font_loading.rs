use super::*;
use boa_engine::object::builtins::{JsArray, JsPromise};

#[derive(Clone)]
struct FontFaceSetRealmBindings {
    event_target_constructor: JsObject,
    font_face_set_prototype: JsObject,
}

pub(super) fn register_font_face_set_interface(context: &mut Context) -> JsResult<()> {
    if context.get_data::<FontFaceSetRealmBindings>().is_some() {
        return Ok(());
    }

    let event_target_constructor = interface_constructor("EventTarget", context)?;
    let font_face_set_constructor = interface_constructor("FontFaceSet", context)?;
    let font_face_set_prototype = interface_prototype(&font_face_set_constructor, context)?;

    for (name, length, function) in [
        (
            "load",
            1,
            font_face_set_load as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
        ),
        ("check", 1, font_face_set_check),
    ] {
        let method =
            FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(function))
                .name(js_string!(name))
                .length(length)
                .constructor(false)
                .build();
        font_face_set_prototype.define_property_or_throw(
            js_string!(name),
            PropertyDescriptor::builder()
                .value(method)
                .writable(true)
                .enumerable(false)
                .configurable(true)
                .build(),
            context,
        )?;
    }

    for (name, getter) in [
        (
            "status",
            font_face_set_status as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
        ),
        ("size", font_face_set_size),
        ("ready", font_face_set_ready),
    ] {
        let getter =
            FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(getter))
                .name(js_string!(name))
                .length(0)
                .constructor(false)
                .build();
        font_face_set_prototype.define_property_or_throw(
            js_string!(name),
            PropertyDescriptor::builder()
                .get(getter)
                .enumerable(true)
                .configurable(true)
                .build(),
            context,
        )?;
    }

    context.insert_data(FontFaceSetRealmBindings {
        event_target_constructor,
        font_face_set_prototype,
    });
    Ok(())
}

fn interface_constructor(name: &str, context: &mut Context) -> JsResult<JsObject> {
    context
        .global_object()
        .get(js_string!(name), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error()
                .with_message("DOM interface constructor missing")
                .into()
        })
}

fn interface_prototype(constructor: &JsObject, context: &mut Context) -> JsResult<JsObject> {
    constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error()
                .with_message("DOM interface prototype missing")
                .into()
        })
}

pub(super) fn document_get_font_face_set(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let document_node = document_root(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    if let Some(fonts) = state.borrow().font_face_sets.get(&document_node) {
        return Ok(fonts.clone().into());
    }

    let bindings = context
        .get_data::<FontFaceSetRealmBindings>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("FontFaceSet interface unavailable"))?;
    let fonts = bindings
        .event_target_constructor
        .construct(&[], None, context)?;
    if !fonts.set_prototype(Some(bindings.font_face_set_prototype)) {
        return Err(JsNativeError::error()
            .with_message("Could not initialize FontFaceSet prototype")
            .into());
    }

    let (ready, resolvers) = JsPromise::new_pending(context);
    {
        let mut state = state.borrow_mut();
        state.font_face_sets.insert(document_node, fonts.clone());
        state
            .font_face_set_readies
            .insert(document_node, ready.into());
    }
    resolvers
        .resolve
        .call(&JsValue::undefined(), &[fonts.clone().into()], context)?;
    Ok(fonts.into())
}

fn font_face_set_owner(this: &JsValue, context: &Context) -> JsResult<NodeId> {
    let object = this
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Illegal FontFaceSet receiver"))?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state
        .borrow()
        .font_face_sets
        .iter()
        .find_map(|(document, candidate)| (candidate == &object).then_some(*document))
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("Illegal FontFaceSet receiver")
                .into()
        });
    document
}

fn font_face_set_status(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    font_face_set_owner(this, context)?;
    Ok(js_string!("loaded").into())
}

fn font_face_set_size(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    font_face_set_owner(this, context)?;
    Ok(JsValue::new(0))
}

fn font_face_set_ready(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let document_node = font_face_set_owner(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let ready = state
        .borrow()
        .font_face_set_readies
        .get(&document_node)
        .cloned();
    ready.map(Into::into).ok_or_else(|| {
        JsNativeError::error()
            .with_message("FontFaceSet ready promise missing")
            .into()
    })
}

fn font_arguments(args: &[JsValue], context: &mut Context) -> JsResult<(String, String)> {
    let Some(font) = args.first() else {
        return Err(JsNativeError::typ()
            .with_message("FontFaceSet method requires a font string")
            .into());
    };
    let font = font.to_string(context)?.to_std_string_escaped();
    let text = match args.get(1) {
        Some(text) => text.to_string(context)?.to_std_string_escaped(),
        None => " ".to_string(),
    };
    Ok((font, text))
}

fn font_syntax_error(context: &mut Context) -> JsResult<JsObject> {
    let constructor = context
        .global_object()
        .get(js_string!("DOMException"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("DOMException constructor missing"))?;
    constructor.construct(
        &[
            js_string!("The supplied font string is invalid").into(),
            js_string!("SyntaxError").into(),
        ],
        None,
        context,
    )
}

fn font_face_set_check(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    font_face_set_owner(this, context)?;
    let (font, _text) = font_arguments(args, context)?;
    if !rix_css::supports_declaration("font", &font) {
        return Err(JsError::from_opaque(font_syntax_error(context)?.into()));
    }
    Ok(JsValue::new(true))
}

fn font_face_set_load(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    font_face_set_owner(this, context)?;
    let (font, _text) = font_arguments(args, context)?;
    if !rix_css::supports_declaration("font", &font) {
        let error = JsError::from_opaque(font_syntax_error(context)?.into());
        return Ok(JsPromise::reject(error, context)?.into());
    }

    let fonts = JsArray::from_iter(std::iter::empty::<JsValue>(), context);
    Ok(JsPromise::resolve(fonts, context)?.into())
}
