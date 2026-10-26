
use super::*;
use rix_core::url::RixUrl;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HistoryChange {
    pub url: String,
    pub kind: HistoryChangeKind,
    pub index: u32,
    pub length: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HistoryChangeKind {
    Push,
    Replace,
    Traverse,
}

struct HistoryEntry {
    url: RixUrl,
    state: JsValue,
}

struct HistoryApiState {
    entries: Vec<HistoryEntry>,
    index: usize,
    object: JsObject,
    scroll_restoration: String,
}

pub(super) fn register_history(context: &mut Context) -> JsResult<()> {
    if context.get_data::<Rc<RefCell<HistoryApiState>>>().is_some() {
        return Ok(());
    }

    context.eval(boa_engine::Source::from_bytes(
        "globalThis.History = class History { constructor() { throw new TypeError('Illegal constructor: History'); } }; Object.defineProperty(History.prototype, Symbol.toStringTag, {value: 'History', configurable: true});",
    ))?;
    let constructor = context
        .global_object()
        .get(js_string!("History"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("History constructor is unavailable"))?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("History prototype is unavailable"))?;

    for (name, length, function) in [
        (
            "back",
            0,
            history_back as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
        ),
        ("forward", 0, history_forward),
        ("go", 0, history_go),
        ("pushState", 2, history_push_state),
        ("replaceState", 2, history_replace_state),
    ] {
        let method =
            FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(function))
                .name(js_string!(name))
                .length(length)
                .build();
        prototype.define_property_or_throw(
            js_string!(name),
            PropertyDescriptor::builder()
                .value(method)
                .writable(true)
                .enumerable(true)
                .configurable(true)
                .build(),
            context,
        )?;
    }

    define_getter(context, &prototype, "length", history_length)?;
    define_getter(context, &prototype, "state", history_state)?;
    let scroll_getter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(history_scroll_restoration_get),
    )
    .name(js_string!("get scrollRestoration"))
    .build();
    let scroll_setter = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(history_scroll_restoration_set),
    )
    .name(js_string!("set scrollRestoration"))
    .length(1)
    .build();
    prototype.define_property_or_throw(
        js_string!("scrollRestoration"),
        PropertyDescriptor::builder()
            .get(scroll_getter)
            .set(scroll_setter)
            .enumerable(true)
            .configurable(true)
            .build(),
        context,
    )?;

    let current_url = context
        .get_data::<Rc<RefCell<WebApiState>>>()
        .and_then(|state| state.borrow().current_url.clone())
        .map_or_else(
            || {
                RixUrl::parse("about:blank")
                    .map_err(|error| JsNativeError::error().with_message(error.to_string()))
            },
            Ok,
        )?;
    let object = ObjectInitializer::new(context).build();
    object.set_prototype(Some(prototype));
    context.register_global_property(
        js_string!("history"),
        object.clone(),
        Attribute::READONLY | Attribute::CONFIGURABLE,
    )?;
    context.insert_data(Rc::new(RefCell::new(HistoryApiState {
        entries: vec![HistoryEntry {
            url: current_url,
            state: JsValue::null(),
        }],
        index: 0,
        object,
        scroll_restoration: "auto".to_string(),
    })));
    Ok(())
}

fn define_getter(
    context: &mut Context,
    prototype: &JsObject,
    name: &str,
    getter: fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
) -> JsResult<()> {
    let getter = FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(getter))
        .name(format!("get {name}"))
        .build();
    prototype
        .define_property_or_throw(
            js_string!(name),
            PropertyDescriptor::builder()
                .get(getter)
                .enumerable(true)
                .configurable(true)
                .build(),
            context,
        )
        .map(|_| ())
}

fn state_for_history(
    this: &JsValue,
    context: &mut Context,
) -> JsResult<Rc<RefCell<HistoryApiState>>> {
    let state = context
        .get_data::<Rc<RefCell<HistoryApiState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("History is unavailable"))?;
    let expected = state.borrow().object.clone();
    if !this
        .as_object()
        .is_some_and(|actual| JsObject::equals(&expected, &actual))
    {
        return Err(JsNativeError::typ()
            .with_message("History operation called on an incompatible receiver")
            .into());
    }
    Ok(state)
}

fn history_length(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let state = state_for_history(this, context)?;
    let length = state.borrow().entries.len() as u32;
    Ok(JsValue::new(length))
}

fn history_state(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let state = state_for_history(this, context)?;
    let value = {
        let history = state.borrow();
        history
            .entries
            .get(history.index)
            .map(|entry| entry.state.clone())
            .unwrap_or_else(JsValue::null)
    };
    crate::structured_clone::clone_serializable_value(&value, context)
}

fn history_scroll_restoration_get(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = state_for_history(this, context)?;
    let value = state.borrow().scroll_restoration.clone();
    Ok(js_string!(value).into())
}

fn history_scroll_restoration_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = state_for_history(this, context)?;
    let value = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    if value != "auto" && value != "manual" {
        return Err(JsNativeError::typ()
            .with_message("scrollRestoration must be 'auto' or 'manual'")
            .into());
    }
    state.borrow_mut().scroll_restoration = value;
    Ok(JsValue::undefined())
}

fn history_push_state(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = state_for_history(this, context)?;
    if args.len() < 2 {
        return Err(JsNativeError::typ()
            .with_message("History.pushState requires at least two arguments")
            .into());
    }
    let serialized =
        crate::structured_clone::clone_serializable_value(args.get_or_undefined(0), context)?;
    let _title = args.get_or_undefined(1).to_string(context)?;
    let current = current_history_url(&state, context)?;
    let url = resolve_history_url(&current, args.get(2), context)?;
    {
        let mut history = state.borrow_mut();
        let next_index = history.index + 1;
        history.entries.truncate(next_index);
        history.entries.push(HistoryEntry {
            url: url.clone(),
            state: serialized,
        });
        history.index = next_index;
    }
    update_document_url(url.clone(), context);
    record_history_change(url.as_str(), HistoryChangeKind::Push, &state, context);
    if current.inner().path() == url.inner().path()
        && current.inner().query() == url.inner().query()
        && current.inner().fragment() != url.inner().fragment()
    {
        let _ = queue_hashchange(&current, &url, context);
    }
    Ok(JsValue::undefined())
}

fn history_replace_state(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = state_for_history(this, context)?;
    if args.len() < 2 {
        return Err(JsNativeError::typ()
            .with_message("History.replaceState requires at least two arguments")
            .into());
    }
    let serialized =
        crate::structured_clone::clone_serializable_value(args.get_or_undefined(0), context)?;
    let _title = args.get_or_undefined(1).to_string(context)?;
    let current = current_history_url(&state, context)?;
    let url = resolve_history_url(&current, args.get(2), context)?;
    {
        let mut history = state.borrow_mut();
        let index = history.index;
        if let Some(entry) = history.entries.get_mut(index) {
            entry.url = url.clone();
            entry.state = serialized;
        }
    }
    update_document_url(url.clone(), context);
    record_history_change(url.as_str(), HistoryChangeKind::Replace, &state, context);
    Ok(JsValue::undefined())
}

fn current_history_url(
    state: &Rc<RefCell<HistoryApiState>>,
    context: &Context,
) -> JsResult<RixUrl> {
    if let Some(url) = context
        .get_data::<Rc<RefCell<WebApiState>>>()
        .and_then(|web| web.borrow().current_url.clone())
    {
        return Ok(url);
    }
    let history = state.borrow();
    history
        .entries
        .get(history.index)
        .map(|entry| entry.url.clone())
        .ok_or_else(|| {
            JsNativeError::error()
                .with_message("History has no current entry")
                .into()
        })
}

fn resolve_history_url(
    current: &RixUrl,
    argument: Option<&JsValue>,
    context: &mut Context,
) -> JsResult<RixUrl> {
    let Some(argument) = argument.filter(|value| !value.is_null() && !value.is_undefined()) else {
        return Ok(current.clone());
    };
    let value = argument.to_string(context)?.to_std_string_escaped();
    let target = match current.join(&value) {
        Ok(url) => url,
        Err(error) => return Err(history_security_error(&error.to_string(), context)?),
    };
    if target.inner().origin() != current.inner().origin() {
        return Err(history_security_error(
            "History URL must be same-origin",
            context,
        )?);
    }
    Ok(target)
}

fn history_security_error(message: &str, context: &mut Context) -> JsResult<JsError> {
    let constructor = context
        .global_object()
        .get(js_string!("DOMException"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("DOMException constructor is unavailable")
        })?;
    let exception = constructor.construct(
        &[
            js_string!(message).into(),
            js_string!("SecurityError").into(),
        ],
        None,
        context,
    )?;
    Ok(JsError::from_opaque(exception.into()))
}

fn update_document_url(url: RixUrl, context: &mut Context) {
    if let Some(web) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        web.borrow_mut().current_url = Some(url);
    }
}

fn history_back(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    history_traverse(this, -1, context)
}

fn history_forward(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    history_traverse(this, 1, context)
}

fn history_go(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let amount = match args.first() {
        None => 0,
        Some(value) => {
            let number = value.to_number(context)?;
            if !number.is_finite() || number == 0.0 {
                0
            } else {
                let modulo = (number.trunc() % 4_294_967_296.0 + 4_294_967_296.0) % 4_294_967_296.0;
                if modulo >= 2_147_483_648.0 {
                    (modulo - 4_294_967_296.0) as i32
                } else {
                    modulo as i32
                }
            }
        }
    };
    history_traverse(this, amount, context)
}

fn history_traverse(this: &JsValue, amount: i32, context: &mut Context) -> JsResult<JsValue> {
    let state = state_for_history(this, context)?;
    if amount == 0 {
        if let Some(web) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
            web.borrow_mut()
                .record_navigation("", PageNavigationKind::Reload);
        }
        return Ok(JsValue::undefined());
    }
    traverse_history_state(&state, amount, context)?;
    Ok(JsValue::undefined())
}

pub fn traverse_session_history(context: &mut Context, amount: i32) -> JsResult<bool> {
    if amount == 0 {
        return Ok(false);
    }
    let Some(state) = context.get_data::<Rc<RefCell<HistoryApiState>>>().cloned() else {
        return Ok(false);
    };
    traverse_history_state(&state, amount, context)
}

fn traverse_history_state(
    state: &Rc<RefCell<HistoryApiState>>,
    amount: i32,
    context: &mut Context,
) -> JsResult<bool> {
    let previous_url = current_history_url(state, context)?;
    let target = {
        let mut history = state.borrow_mut();
        let target_index = (history.index as i64) + i64::from(amount);
        if target_index < 0 || target_index >= history.entries.len() as i64 {
            return Ok(false);
        }
        history.index = target_index as usize;
        history
            .entries
            .get(history.index)
            .map(|entry| (entry.url.clone(), entry.state.clone()))
    };
    let Some((url, value)) = target else {
        return Ok(false);
    };
    update_document_url(url.clone(), context);
    record_history_change(url.as_str(), HistoryChangeKind::Traverse, state, context);
    dispatch_popstate(value, context)?;
    if previous_url.inner().fragment() != url.inner().fragment() {
        queue_hashchange(&previous_url, &url, context)?;
    }
    Ok(true)
}

fn record_history_change(
    url: &str,
    kind: HistoryChangeKind,
    state: &Rc<RefCell<HistoryApiState>>,
    context: &mut Context,
) {
    let (index, length) = {
        let history = state.borrow();
        (history.index as u32, history.entries.len() as u32)
    };
    if let Some(web) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        web.borrow_mut()
            .record_history_change(url, kind, index, length);
    }
}

fn queue_hashchange(old_url: &RixUrl, new_url: &RixUrl, context: &mut Context) -> JsResult<()> {
    let Some(web) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() else {
        return Ok(());
    };
    let dispatcher = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(dispatch_hashchange_task),
    )
    .name(js_string!("hashchange"))
    .build();
    web.borrow_mut().schedule_timer(
        TimerCallback::Function(dispatcher.into()),
        vec![
            js_string!(old_url.as_str()).into(),
            js_string!(new_url.as_str()).into(),
        ],
        Duration::ZERO,
        None,
    );
    Ok(())
}

fn dispatch_hashchange_task(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let old_url = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let new_url = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    let constructor = context
        .global_object()
        .get(js_string!("HashChangeEvent"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("HashChangeEvent constructor is unavailable")
        })?;
    let init = ObjectInitializer::new(context)
        .property(js_string!("oldURL"), js_string!(old_url), Attribute::all())
        .property(js_string!("newURL"), js_string!(new_url), Attribute::all())
        .build();
    let event = constructor.construct(
        &[js_string!("hashchange").into(), init.into()],
        None,
        context,
    )?;
    let global = context.global_object();
    let dispatch = global
        .get(js_string!("dispatchEvent"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Window.dispatchEvent is unavailable"))?;
    dispatch.call(&global.into(), &[event.into()], context)?;
    Ok(JsValue::undefined())
}

fn dispatch_popstate(value: JsValue, context: &mut Context) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!("PopStateEvent"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("PopStateEvent constructor is unavailable")
        })?;
    let init = ObjectInitializer::new(context)
        .property(js_string!("state"), value, Attribute::all())
        .build();
    let event =
        constructor.construct(&[js_string!("popstate").into(), init.into()], None, context)?;
    let global = context.global_object();
    let dispatch = global
        .get(js_string!("dispatchEvent"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Window.dispatchEvent is unavailable"))?;
    dispatch.call(&global.into(), &[event.into()], context)?;
    Ok(())
}
