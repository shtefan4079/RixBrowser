
use super::*;
use url::Url;

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

#[derive(Clone, Copy)]
enum UrlComponent {
    Href,
    Origin,
    Protocol,
    Username,
    Password,
    Host,
    Hostname,
    Port,
    Pathname,
    Search,
    Hash,
}

impl UrlComponent {
    fn name(self) -> &'static str {
        match self {
            Self::Href => "href",
            Self::Origin => "origin",
            Self::Protocol => "protocol",
            Self::Username => "username",
            Self::Password => "password",
            Self::Host => "host",
            Self::Hostname => "hostname",
            Self::Port => "port",
            Self::Pathname => "pathname",
            Self::Search => "search",
            Self::Hash => "hash",
        }
    }

    fn writable(self) -> bool {
        !matches!(self, Self::Origin)
    }
}

pub(super) fn register_hyperlink_interfaces(context: &mut Context) -> JsResult<()> {
    for interface in ["HTMLAnchorElement", "HTMLAreaElement"] {
        let constructor = context
            .global_object()
            .get(js_string!(interface), context)?
            .as_object()
            .ok_or_else(|| {
                JsNativeError::error().with_message(format!("{interface} is unavailable"))
            })?;
        let prototype = constructor
            .get(js_string!("prototype"), context)?
            .as_object()
            .ok_or_else(|| {
                JsNativeError::error().with_message(format!("{interface} prototype is unavailable"))
            })?;

        for component in [
            UrlComponent::Href,
            UrlComponent::Origin,
            UrlComponent::Protocol,
            UrlComponent::Username,
            UrlComponent::Password,
            UrlComponent::Host,
            UrlComponent::Hostname,
            UrlComponent::Port,
            UrlComponent::Pathname,
            UrlComponent::Search,
            UrlComponent::Hash,
        ] {
            let name = component.name();
            let getter = FunctionObjectBuilder::new(
                context.realm(),
                NativeFunction::from_copy_closure(move |this, _args, context| {
                    get_component(this, component, context)
                }),
            )
            .name(js_string!(format!("get {name}")))
            .build();
            let mut descriptor = PropertyDescriptor::builder()
                .get(getter)
                .enumerable(true)
                .configurable(true);
            if component.writable() {
                let setter = FunctionObjectBuilder::new(
                    context.realm(),
                    NativeFunction::from_copy_closure(move |this, args, context| {
                        set_component(this, component, args, context)
                    }),
                )
                .name(js_string!(format!("set {name}")))
                .length(1)
                .build();
                descriptor = descriptor.set(setter);
            }
            prototype.define_property_or_throw(js_string!(name), descriptor.build(), context)?;
        }

        let to_string = FunctionObjectBuilder::new(
            context.realm(),
            NativeFunction::from_fn_ptr(hyperlink_to_string),
        )
        .name(js_string!("toString"))
        .length(0)
        .build();
        prototype.define_property_or_throw(
            js_string!("toString"),
            PropertyDescriptor::builder()
                .value(to_string)
                .writable(true)
                .configurable(true)
                .build(),
            context,
        )?;
    }
    Ok(())
}

pub(super) fn register_script_src_property(context: &mut Context) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!("HTMLScriptElement"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("HTMLScriptElement is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("HTMLScriptElement prototype is unavailable")
        })?;
    let getter =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(script_src_get))
            .name(js_string!("get src"))
            .build();
    let setter =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(script_src_set))
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
    Ok(())
}

pub(super) fn register_script_async_property(context: &mut Context) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!("HTMLScriptElement"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("HTMLScriptElement is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("HTMLScriptElement prototype is unavailable")
        })?;
    let getter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(script_async_get),
    )
    .name(js_string!("get async"))
    .build();
    let setter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(script_async_set),
    )
    .name(js_string!("set async"))
    .length(1)
    .build();
    prototype.define_property_or_throw(
        js_string!("async"),
        PropertyDescriptor::builder()
            .get(getter)
            .set(setter)
            .enumerable(true)
            .configurable(true)
            .build(),
        context,
    )?;
    Ok(())
}

pub(super) fn register_script_defer_property(context: &mut Context) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!("HTMLScriptElement"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("HTMLScriptElement is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("HTMLScriptElement prototype is unavailable")
        })?;
    let getter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(script_defer_get),
    )
    .name(js_string!("get defer"))
    .build();
    let setter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(script_defer_set),
    )
    .name(js_string!("set defer"))
    .length(1)
    .build();
    prototype.define_property_or_throw(
        js_string!("defer"),
        PropertyDescriptor::builder()
            .get(getter)
            .set(setter)
            .enumerable(true)
            .configurable(true)
            .build(),
        context,
    )?;
    Ok(())
}

pub(super) fn register_script_type_property(context: &mut Context) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!("HTMLScriptElement"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("HTMLScriptElement is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("HTMLScriptElement prototype is unavailable")
        })?;
    let getter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(script_type_get),
    )
    .name(js_string!("get type"))
    .build();
    let setter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(script_type_set),
    )
    .name(js_string!("set type"))
    .length(1)
    .build();
    prototype.define_property_or_throw(
        js_string!("type"),
        PropertyDescriptor::builder()
            .get(getter)
            .set(setter)
            .enumerable(true)
            .configurable(true)
            .build(),
        context,
    )?;
    Ok(())
}

pub(super) fn register_script_text_property(context: &mut Context) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!("HTMLScriptElement"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("HTMLScriptElement is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("HTMLScriptElement prototype is unavailable")
        })?;
    let getter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(script_text_get),
    )
    .name(js_string!("get text"))
    .build();
    let setter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(script_text_set),
    )
    .name(js_string!("set text"))
    .length(1)
    .build();
    prototype.define_property_or_throw(
        js_string!("text"),
        PropertyDescriptor::builder()
            .get(getter)
            .set(setter)
            .enumerable(true)
            .configurable(true)
            .build(),
        context,
    )?;
    Ok(())
}

pub(super) fn register_script_nomodule_property(context: &mut Context) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!("HTMLScriptElement"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("HTMLScriptElement is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("HTMLScriptElement prototype is unavailable")
        })?;
    let getter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(script_nomodule_get),
    )
    .name(js_string!("get noModule"))
    .build();
    let setter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(script_nomodule_set),
    )
    .name(js_string!("set noModule"))
    .length(1)
    .build();
    prototype.define_property_or_throw(
        js_string!("noModule"),
        PropertyDescriptor::builder()
            .get(getter)
            .set(setter)
            .enumerable(true)
            .configurable(true)
            .build(),
        context,
    )?;
    Ok(())
}

fn hyperlink_node(this: &JsValue, context: &mut Context) -> JsResult<NodeId> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let is_hyperlink = {
        let state = state.borrow();
        let document = state.document.borrow();
        matches!(document.get(node), Some(NodeData::Element(element))
            if &*element.name.ns == HTML_NAMESPACE
                && matches!(&*element.name.local, "a" | "area"))
    };
    if !is_hyperlink {
        return Err(JsNativeError::typ()
            .with_message("HTML hyperlink URL property called on an incompatible receiver")
            .into());
    }
    Ok(node)
}

fn href_attribute(node: NodeId, context: &mut Context) -> JsResult<Option<String>> {
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let state = state.borrow();
    let document = state.document.borrow();
    Ok(match document.get(node) {
        Some(NodeData::Element(element)) => element.get_attribute("href").map(str::to_owned),
        _ => None,
    })
}

fn script_node(this: &JsValue, context: &mut Context) -> JsResult<NodeId> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let is_script = {
        let state = state.borrow();
        let document = state.document.borrow();
        matches!(document.get(node), Some(NodeData::Element(element))
            if &*element.name.ns == HTML_NAMESPACE && &*element.name.local == "script")
    };
    if !is_script {
        return Err(JsNativeError::typ()
            .with_message("HTMLScriptElement.src called on an incompatible receiver")
            .into());
    }
    Ok(node)
}

fn script_src_get(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = script_node(this, context)?;
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
    let resolved = document_base_url(node, context)
        .and_then(|base| base.join(&raw).ok())
        .or_else(|| Url::parse(&raw).ok());
    Ok(js_string!(resolved.map(|url| url.to_string()).unwrap_or(raw)).into())
}

fn script_src_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    script_node(this, context)?;
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

fn script_async_get(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = script_node(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let is_async = state.borrow().script_async(node);
    Ok(is_async.into())
}

fn script_async_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = script_node(this, context)?;
    let is_async = args.get_or_undefined(0).to_boolean();
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    state.borrow_mut().script_force_async.insert(node, is_async);
    if is_async {
        element_set_attribute(
            this,
            &[js_string!("async").into(), js_string!("").into()],
            context,
        )
    } else {
        element_remove_attribute(this, &[js_string!("async").into()], context)
    }
}

fn script_defer_get(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = script_node(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let has_defer = state
        .borrow()
        .document
        .borrow()
        .get(node)
        .and_then(|data| match data {
            NodeData::Element(element) => Some(element.get_attribute("defer").is_some()),
            _ => None,
        })
        .unwrap_or(false);
    Ok(has_defer.into())
}

fn script_defer_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    script_node(this, context)?;
    let is_defer = args.get_or_undefined(0).to_boolean();
    if is_defer {
        element_set_attribute(
            this,
            &[js_string!("defer").into(), js_string!("").into()],
            context,
        )
    } else {
        element_remove_attribute(this, &[js_string!("defer").into()], context)
    }
}

fn script_type_get(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = script_node(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let raw = state
        .borrow()
        .document
        .borrow()
        .get(node)
        .and_then(|data| match data {
            NodeData::Element(element) => element.get_attribute("type").map(str::to_string),
            _ => None,
        })
        .unwrap_or_default();
    Ok(js_string!(raw).into())
}

fn script_type_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    script_node(this, context)?;
    let value = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    element_set_attribute(
        this,
        &[js_string!("type").into(), js_string!(value).into()],
        context,
    )
}

fn script_text_get(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    script_node(this, context)?;
    super::document::node_get_text_content(this, args, context)
}

fn script_text_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    script_node(this, context)?;
    super::document::node_set_text_content(this, args, context)
}

fn script_nomodule_get(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = script_node(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("DOM bridge is unavailable"))?;
    let has_nomodule = state
        .borrow()
        .document
        .borrow()
        .get(node)
        .and_then(|data| match data {
            NodeData::Element(element) => Some(element.get_attribute("nomodule").is_some()),
            _ => None,
        })
        .unwrap_or(false);
    Ok(has_nomodule.into())
}

fn script_nomodule_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    script_node(this, context)?;
    let is_nomodule = args.get_or_undefined(0).to_boolean();
    if is_nomodule {
        element_set_attribute(
            this,
            &[js_string!("nomodule").into(), js_string!("").into()],
            context,
        )
    } else {
        element_remove_attribute(this, &[js_string!("nomodule").into()], context)
    }
}

pub(super) fn document_base_url(node: NodeId, context: &mut Context) -> Option<Url> {
    let dom_state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned()?;
    let current_url = context
        .get_data::<Rc<RefCell<crate::web_api::WebApiState>>>()
        .and_then(|state| state.borrow().current_url.clone())
        .map(|url| url.inner().clone());

    let (document_id, main_document, base_href) =
        {
            let state = dom_state.borrow();
            let owner = state.owner_document(node).unwrap_or(state.main_document);
            let associated_document = state.template_contents_documents.iter().find_map(
                |(associated, template_document)| {
                    (*template_document == owner).then_some(*associated)
                },
            );
            let document_id = associated_document.unwrap_or(owner);
            let document = state.document.borrow();
            let base_href = document
                .descendants(document_id)
                .find_map(|candidate| match document.get(candidate) {
                    Some(NodeData::Element(element))
                        if &*element.name.ns == HTML_NAMESPACE
                            && &*element.name.local == "base" =>
                    {
                        element.get_attribute("href").map(str::to_owned)
                    }
                    _ => None,
                });
            (document_id, state.main_document, base_href)
        };

    let fallback = if document_id == main_document {
        current_url.or_else(|| Url::parse("about:blank").ok())
    } else {
        Url::parse("about:blank").ok()
    }?;

    Some(match base_href {
        Some(href) => fallback.join(&href).unwrap_or(fallback),
        None => fallback,
    })
}

fn parse_hyperlink_url(node: NodeId, context: &mut Context) -> JsResult<Option<Url>> {
    let Some(href) = href_attribute(node, context)? else {
        return Ok(None);
    };
    let parsed = document_base_url(node, context)
        .and_then(|base| base.join(&href).ok())
        .or_else(|| Url::parse(&href).ok());
    Ok(parsed)
}

fn get_component(
    this: &JsValue,
    component: UrlComponent,
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = hyperlink_node(this, context)?;
    let attribute = href_attribute(node, context)?;
    if matches!(component, UrlComponent::Href) {
        return Ok(js_string!(parse_hyperlink_url(node, context)?
            .map(|url| url.to_string())
            .or(attribute)
            .unwrap_or_default())
        .into());
    }
    let Some(url) = parse_hyperlink_url(node, context)? else {
        return Ok(js_string!("").into());
    };
    let value = match component {
        UrlComponent::Href => String::new(),
        UrlComponent::Origin => url.origin().ascii_serialization(),
        UrlComponent::Protocol => format!("{}:", url.scheme()),
        UrlComponent::Username => url.username().to_owned(),
        UrlComponent::Password => url.password().unwrap_or_default().to_owned(),
        UrlComponent::Host => {
            let mut host = url.host().map(|host| host.to_string()).unwrap_or_default();
            if let Some(port) = url.port() {
                host.push(':');
                host.push_str(&port.to_string());
            }
            host
        }
        UrlComponent::Hostname => url.host().map(|host| host.to_string()).unwrap_or_default(),
        UrlComponent::Port => url.port().map(|port| port.to_string()).unwrap_or_default(),
        UrlComponent::Pathname => url.path().to_owned(),
        UrlComponent::Search => url
            .query()
            .filter(|query| !query.is_empty())
            .map(|query| format!("?{query}"))
            .unwrap_or_default(),
        UrlComponent::Hash => url
            .fragment()
            .filter(|fragment| !fragment.is_empty())
            .map(|fragment| format!("#{fragment}"))
            .unwrap_or_default(),
    };
    Ok(js_string!(value).into())
}

fn set_component(
    this: &JsValue,
    component: UrlComponent,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = hyperlink_node(this, context)?;
    let value = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    if matches!(component, UrlComponent::Href) {
        return set_href_attribute(this, &value, context);
    }

    let Some(mut url) = parse_hyperlink_url(node, context)? else {
        return Ok(JsValue::undefined());
    };
    let updated = match component {
        UrlComponent::Href | UrlComponent::Origin => false,
        UrlComponent::Protocol => url
            .set_scheme(value.strip_suffix(':').unwrap_or(&value))
            .is_ok(),
        UrlComponent::Username => url.set_username(&value).is_ok(),
        UrlComponent::Password => url.set_password(Some(&value)).is_ok(),
        UrlComponent::Host => set_host(&mut url, &value),
        UrlComponent::Hostname => set_hostname(&mut url, &value),
        UrlComponent::Port => set_port(&mut url, &value),
        UrlComponent::Pathname => {
            url.set_path(&value);
            true
        }
        UrlComponent::Search => {
            let query = if value.is_empty() {
                None
            } else {
                Some(value.strip_prefix('?').unwrap_or(&value))
            };
            url.set_query(query);
            true
        }
        UrlComponent::Hash => {
            let fragment = if value.is_empty() {
                None
            } else {
                Some(value.strip_prefix('#').unwrap_or(&value))
            };
            url.set_fragment(fragment);
            true
        }
    };
    if updated {
        set_href_attribute(this, url.as_str(), context)
    } else {
        Ok(JsValue::undefined())
    }
}

fn set_href_attribute(this: &JsValue, value: &str, context: &mut Context) -> JsResult<JsValue> {
    element_set_attribute(
        this,
        &[js_string!("href").into(), js_string!(value).into()],
        context,
    )
}

fn set_host(url: &mut Url, value: &str) -> bool {
    let Some((host, port)) = split_host_port(value) else {
        return false;
    };
    let mut candidate = url.clone();
    if candidate.set_host(Some(host)).is_err() {
        return false;
    }
    if let Some(port) = port {
        if candidate.set_port(port).is_err() {
            return false;
        }
    }
    *url = candidate;
    true
}

fn split_host_port(value: &str) -> Option<(&str, Option<Option<u16>>)> {
    if value.starts_with('[') {
        let closing = value.find(']')?;
        let host_end = closing + 1;
        let suffix = &value[host_end..];
        if suffix.is_empty() {
            return Some((value, None));
        }
        let port = suffix.strip_prefix(':')?;
        return Some((
            &value[..host_end],
            Some(if port.is_empty() {
                None
            } else {
                Some(port.parse().ok()?)
            }),
        ));
    }
    match value.matches(':').count() {
        0 => Some((value, None)),
        1 => {
            let (host, port) = value.split_once(':')?;
            Some((
                host,
                Some(if port.is_empty() {
                    None
                } else {
                    Some(port.parse().ok()?)
                }),
            ))
        }
        _ => None,
    }
}

fn set_hostname(url: &mut Url, value: &str) -> bool {
    if value.contains(':') && !(value.starts_with('[') && value.ends_with(']')) {
        return false;
    }
    url.set_host(Some(value)).is_ok()
}

fn set_port(url: &mut Url, value: &str) -> bool {
    if value.is_empty() {
        return url.set_port(None).is_ok();
    }
    let Ok(port) = value.parse::<u16>() else {
        return false;
    };
    url.set_port(Some(port)).is_ok()
}

fn hyperlink_to_string(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    get_component(this, UrlComponent::Href, context)
}
