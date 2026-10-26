
use super::*;
use url::Url;

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

pub(super) fn register_html_image_properties(context: &mut Context) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!("HTMLImageElement"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("HTMLImageElement is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("HTMLImageElement prototype is unavailable")
        })?;
    let getter =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(image_src_get))
            .name(js_string!("get src"))
            .build();
    let setter =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(image_src_set))
            .name(js_string!("set src"))
            .length(1)
            .build();
    prototype.define_property_or_throw(
        js_string!("src"),
        PropertyDescriptor::builder()
            .get(getter)
            .set(setter)
            .enumerable(true)
            .configurable(true)
            .build(),
        context,
    )?;
    for (name, getter) in [
        ("complete", image_complete_get as ImageGetter),
        ("naturalWidth", image_natural_width_get),
        ("naturalHeight", image_natural_height_get),
        ("currentSrc", image_current_src_get),
    ] {
        define_image_getter(context, &prototype, name, getter)?;
    }
    Ok(())
}

type ImageGetter = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

fn define_image_getter(
    context: &mut Context,
    prototype: &JsObject,
    name: &str,
    getter: ImageGetter,
) -> JsResult<()> {
    let getter =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(getter))
            .name(js_string!(format!("get {name}")))
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
    Ok(())
}

fn image_node(this: &JsValue, context: &mut Context) -> JsResult<NodeId> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let is_image = {
        let state = state.borrow();
        let document = state.document.borrow();
        matches!(document.get(node), Some(NodeData::Element(element))
            if &*element.name.ns == HTML_NAMESPACE && &*element.name.local == "img")
    };
    if !is_image {
        return Err(JsNativeError::typ()
            .with_message("HTMLImageElement member called on an incompatible receiver")
            .into());
    }
    Ok(node)
}

fn image_source_attributes(
    node: NodeId,
    context: &mut Context,
) -> JsResult<(Option<String>, Option<String>)> {
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let state = state.borrow();
    let document = state.document.borrow();
    let Some(NodeData::Element(element)) = document.get(node) else {
        return Ok((None, None));
    };
    Ok((
        element.get_attribute("src").map(str::to_owned),
        element.get_attribute("srcset").map(str::to_owned),
    ))
}

fn image_request_url(node: NodeId, context: &mut Context) -> JsResult<Option<String>> {
    let (src, srcset) = image_source_attributes(node, context)?;
    let Some(source) = rix_dom::select_image_source(src.as_deref(), srcset.as_deref()) else {
        return Ok(None);
    };
    if source.starts_with("data:image/") {
        return Ok(Some(source.to_string()));
    }
    let resolved = super::hyperlink::document_base_url(node, context)
        .and_then(|base| base.join(source).ok())
        .or_else(|| Url::parse(source).ok());
    Ok(Some(
        resolved
            .map(|url| url.to_string())
            .unwrap_or_else(|| source.to_string()),
    ))
}

fn image_request_state(node: NodeId, context: &mut Context) -> JsResult<ImageLoadState> {
    let Some(url) = image_request_url(node, context)? else {
        return Ok(ImageLoadState::Pending);
    };
    Ok(layout::image_load_state(context, &url))
}

fn image_complete_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = image_node(this, context)?;
    let (src, srcset) = image_source_attributes(node, context)?;
    let no_request = match (src.as_deref(), srcset.as_deref()) {
        (None, None) => true,
        (Some(""), None) => true,
        _ => false,
    };
    if no_request {
        return Ok(true.into());
    }
    Ok(matches!(
        image_request_state(node, context)?,
        ImageLoadState::Loaded { .. } | ImageLoadState::Broken
    )
    .into())
}

fn image_natural_size(this: &JsValue, context: &mut Context) -> JsResult<Option<(u32, u32)>> {
    let node = image_node(this, context)?;
    match image_request_state(node, context)? {
        ImageLoadState::Loaded { width, height } => Ok(Some((
            width.max(0.0).round() as u32,
            height.max(0.0).round() as u32,
        ))),
        _ => Ok(None),
    }
}

fn image_natural_width_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::new(
        image_natural_size(this, context)?.map_or(0, |(width, _)| width),
    ))
}

fn image_natural_height_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::new(
        image_natural_size(this, context)?.map_or(0, |(_, height)| height),
    ))
}

fn image_current_src_get(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = image_node(this, context)?;
    Ok(js_string!(image_request_url(node, context)?.unwrap_or_default()).into())
}

fn image_src_get(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = image_node(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let raw = {
        let state = state.borrow();
        let document = state.document.borrow();
        match document.get(node) {
            Some(NodeData::Element(element)) => element.get_attribute("src").map(str::to_owned),
            _ => None,
        }
    };
    let Some(raw) = raw else {
        return Ok(js_string!("").into());
    };
    let resolved = super::hyperlink::document_base_url(node, context)
        .and_then(|base| base.join(&raw).ok())
        .or_else(|| Url::parse(&raw).ok());
    Ok(js_string!(resolved.map(|url| url.to_string()).unwrap_or(raw)).into())
}

fn image_src_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    image_node(this, context)?;
    let value = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    element_set_attribute(
        this,
        &[js_string!("src").into(), js_string!(value).into()],
        context,
    )
}
