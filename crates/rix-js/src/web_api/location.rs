use super::*;
use rix_core::BrowserProfile;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageNavigationRequest {
    pub url: String,
    pub kind: PageNavigationKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PageNavigationKind {
    Assign,
    Replace,
    Reload,
}

pub(super) fn resolve_navigation_url(state: &WebApiState, input: &str) -> String {
    let trimmed = input.trim();
    state
        .current_url
        .as_ref()
        .and_then(|base| base.join(trimmed).ok())
        .map(|url| url.as_str().to_string())
        .unwrap_or_else(|| trimmed.to_string())
}

pub(super) fn register_navigator(context: &mut Context) -> Result<(), JsError> {
    let profile = BrowserProfile::system_default();
    let mut nav_builder = ObjectInitializer::new(context);
    nav_builder.property(
        js_string!("userAgent"),
        js_string!(profile.user_agent()),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    nav_builder.property(
        js_string!("language"),
        js_string!(profile.language()),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    nav_builder.property(
        js_string!("platform"),
        js_string!(profile.platform()),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    nav_builder.property(
        js_string!("hardwareConcurrency"),
        JsValue::new(profile.hardware_concurrency() as u32),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    nav_builder.property(
        js_string!("maxTouchPoints"),
        JsValue::new(0),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    nav_builder.property(
        js_string!("webdriver"),
        JsValue::new(false),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    nav_builder.property(
        js_string!("onLine"),
        JsValue::new(true),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    nav_builder.property(
        js_string!("cookieEnabled"),
        JsValue::new(true),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    let nav = nav_builder.build();

    let langs = boa_engine::object::builtins::JsArray::from_iter(
        profile
            .languages()
            .iter()
            .map(|language| js_string!(language.as_str()).into()),
        context,
    );
    nav.set(js_string!("languages"), langs, false, context)?;

    context.register_global_property(
        js_string!("navigator"),
        nav,
        Attribute::READONLY | Attribute::CONFIGURABLE,
    )?;

    Ok(())
}

pub(super) fn current_url_string(state: &Rc<RefCell<WebApiState>>) -> String {
    state
        .borrow()
        .current_url
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_else(|| "about:blank".to_string())
}

pub(super) fn register_location(context: &mut Context) -> Result<(), JsError> {
    let state = context.get_data::<Rc<RefCell<WebApiState>>>().cloned();
    let url_str = state
        .as_ref()
        .map(current_url_string)
        .unwrap_or_else(|| "about:blank".to_string());

    let mut loc_builder = ObjectInitializer::new(context);
    loc_builder.property(
        js_string!("href"),
        js_string!(url_str),
        Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE,
    );

    loc_builder.function(
        NativeFunction::from_fn_ptr(location_assign_fn),
        js_string!("assign"),
        1,
    );
    loc_builder.function(
        NativeFunction::from_fn_ptr(location_replace_fn),
        js_string!("replace"),
        1,
    );
    loc_builder.function(
        NativeFunction::from_fn_ptr(location_reload_fn),
        js_string!("reload"),
        0,
    );
    loc_builder.function(
        NativeFunction::from_fn_ptr(location_get_part_fn),
        js_string!("__rixGet"),
        1,
    );
    loc_builder.function(
        NativeFunction::from_fn_ptr(location_set_href_fn),
        js_string!("__rixSetHref"),
        1,
    );

    let loc = loc_builder.build();
    context.register_global_property(
        js_string!("location"),
        loc,
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;

    let live_script = r#"
        (function(loc) {
            if (!loc || typeof loc.__rixGet !== "function") return;
            function defineLive(name, get, set) {
                try {
                    Object.defineProperty(loc, name, {
                        get: get, set: set,
                        enumerable: true, configurable: true
                    });
                } catch (e) {}
            }
            const get = (part) => function() { return loc.__rixGet(part); };
            defineLive("href", get("href"), function(v) { loc.__rixSetHref(String(v)); });
            defineLive("protocol", get("protocol"), undefined);
            defineLive("host", get("host"), undefined);
            defineLive("hostname", get("hostname"), undefined);
            defineLive("port", get("port"), undefined);
            defineLive("pathname", get("pathname"), undefined);
            defineLive("search", get("search"), undefined);
            defineLive("hash", get("hash"), undefined);
            defineLive("origin", get("origin"), undefined);
            loc.toString = function() { return loc.__rixGet("href"); };
            loc.valueOf = function() { return loc.__rixGet("href"); };
        })(globalThis.location);
    "#;
    let _ = context.eval(boa_engine::Source::from_bytes(live_script.as_bytes()));

    Ok(())
}

pub(super) fn location_get_part_fn(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let part = args
        .get_or_undefined(0)
        .to_string(context)
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_default();
    let state = context.get_data::<Rc<RefCell<WebApiState>>>().cloned();
    let href = state
        .as_ref()
        .map(current_url_string)
        .unwrap_or_else(|| "about:blank".to_string());
    let parsed = url::Url::parse(&href).ok();
    let out = match part.as_str() {
        "href" => href.clone(),
        "protocol" => parsed
            .as_ref()
            .map(|u| format!("{}:", u.scheme()))
            .unwrap_or_default(),
        "host" => parsed
            .as_ref()
            .map(|u| {
                u.host_str()
                    .map(|h| match u.port() {
                        Some(p) => format!("{h}:{p}"),
                        None => h.to_string(),
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default(),
        "hostname" => parsed
            .as_ref()
            .and_then(|u| u.host_str())
            .unwrap_or_default()
            .to_string(),
        "port" => parsed
            .as_ref()
            .and_then(|u| u.port())
            .map(|p| p.to_string())
            .unwrap_or_default(),
        "pathname" => parsed
            .as_ref()
            .map(|u| u.path().to_string())
            .unwrap_or_default(),
        "search" => parsed
            .as_ref()
            .and_then(|u| u.query())
            .map(|q| format!("?{q}"))
            .unwrap_or_default(),
        "hash" => parsed
            .as_ref()
            .and_then(|u| {
                let f = u.fragment().unwrap_or("");
                if f.is_empty() {
                    None
                } else {
                    Some(format!("#{f}"))
                }
            })
            .unwrap_or_default(),
        "origin" => parsed
            .as_ref()
            .map(|u| match u.scheme() {
                "http" | "https" => format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")),
                _ => "null".to_string(),
            })
            .unwrap_or_else(|| "null".to_string()),
        _ => String::new(),
    };
    Ok(js_string!(out).into())
}

pub(super) fn location_set_href_fn(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let url = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    if let Some(state) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        state
            .borrow_mut()
            .record_navigation(&url, PageNavigationKind::Assign);
    }
    Ok(JsValue::undefined())
}

pub(super) fn location_assign_fn(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let url = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    if let Some(state) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        state
            .borrow_mut()
            .record_navigation(&url, PageNavigationKind::Assign);
    }
    Ok(JsValue::undefined())
}

pub(super) fn location_replace_fn(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let url = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    if let Some(state) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        state
            .borrow_mut()
            .record_navigation(&url, PageNavigationKind::Replace);
    }
    Ok(JsValue::undefined())
}

pub(super) fn location_reload_fn(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = context.get_data::<Rc<RefCell<WebApiState>>>().cloned();
    if let Some(state) = state {
        state
            .borrow_mut()
            .record_navigation("", PageNavigationKind::Reload);
    }
    Ok(JsValue::undefined())
}
