
use boa_engine::{
    js_string, native_function::NativeFunction, object::ObjectInitializer, property::Attribute,
    Context, JsArgs, JsError, JsResult, JsValue,
};
use rix_core::url::{RixScheme, RixUrl};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabDescriptor {
    pub id: u32,
    pub title: String,
    pub url: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationCommand {
    GoTo(String),
    Back,
    Forward,
    Reload,
}

pub struct PrivilegedHostState {
    pub current_url: Option<RixUrl>,
    pub tabs: Vec<TabDescriptor>,
    pub next_tab_id: u32,
    pub active_tab_id: u32,
    pub navigation_history: Vec<NavigationCommand>,
    pub posted_messages: Vec<String>,
    pub requested_theme: Option<String>,
    pub requested_tab_orientation: Option<String>,
}

impl PrivilegedHostState {
    pub fn new(current_url: Option<RixUrl>) -> Self {
        let initial_tab = TabDescriptor {
            id: 1,
            title: "New Tab".to_string(),
            url: "about:blank".to_string(),
            is_active: true,
        };
        Self {
            current_url,
            tabs: vec![initial_tab],
            next_tab_id: 2,
            active_tab_id: 1,
            navigation_history: Vec::new(),
            posted_messages: Vec::new(),
            requested_theme: None,
            requested_tab_orientation: None,
        }
    }

    pub fn is_privileged(&self) -> bool {
        if let Some(ref url) = self.current_url {
            url.scheme() == RixScheme::Rix && url.host_str() == Some("chrome")
        } else {
            false
        }
    }

    pub fn create_tab(&mut self, url: String) -> u32 {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        for tab in &mut self.tabs {
            tab.is_active = false;
        }
        self.tabs.push(TabDescriptor {
            id,
            title: "New Tab".to_string(),
            url,
            is_active: true,
        });
        self.active_tab_id = id;
        id
    }

    pub fn close_tab(&mut self, id: u32) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        let pos = self.tabs.iter().position(|t| t.id == id);
        if let Some(index) = pos {
            self.tabs.remove(index);
            if self.active_tab_id == id {
                if let Some(last) = self.tabs.last_mut() {
                    last.is_active = true;
                    self.active_tab_id = last.id;
                }
            }
            true
        } else {
            false
        }
    }

    pub fn switch_tab(&mut self, id: u32) -> bool {
        let found = self.tabs.iter().any(|t| t.id == id);
        if found {
            for tab in &mut self.tabs {
                tab.is_active = tab.id == id;
            }
            self.active_tab_id = id;
            true
        } else {
            false
        }
    }
}

pub fn register_privileged_api_if_eligible(
    context: &mut Context,
    state: Rc<RefCell<PrivilegedHostState>>,
) -> Result<bool, JsError> {
    if !state.borrow().is_privileged() {
        return Ok(false);
    }

    context.insert_data(state);

    let mut tabs_builder = ObjectInitializer::new(context);
    tabs_builder.function(
        NativeFunction::from_fn_ptr(rix_tabs_create),
        js_string!("create"),
        1,
    );
    tabs_builder.function(
        NativeFunction::from_fn_ptr(rix_tabs_close),
        js_string!("close"),
        1,
    );
    tabs_builder.function(
        NativeFunction::from_fn_ptr(rix_tabs_switch),
        js_string!("switch"),
        1,
    );
    tabs_builder.function(
        NativeFunction::from_fn_ptr(rix_tabs_list),
        js_string!("list"),
        0,
    );
    let tabs_obj = tabs_builder.build();

    let mut nav_builder = ObjectInitializer::new(context);
    nav_builder.function(
        NativeFunction::from_fn_ptr(rix_navigation_go_to),
        js_string!("goTo"),
        1,
    );
    nav_builder.function(
        NativeFunction::from_fn_ptr(rix_navigation_back),
        js_string!("back"),
        0,
    );
    nav_builder.function(
        NativeFunction::from_fn_ptr(rix_navigation_forward),
        js_string!("forward"),
        0,
    );
    nav_builder.function(
        NativeFunction::from_fn_ptr(rix_navigation_reload),
        js_string!("reload"),
        0,
    );
    let nav_obj = nav_builder.build();

    let mut chrome_builder = ObjectInitializer::new(context);
    chrome_builder.function(
        NativeFunction::from_fn_ptr(rix_chrome_set_theme),
        js_string!("setTheme"),
        1,
    );
    chrome_builder.function(
        NativeFunction::from_fn_ptr(rix_chrome_set_tab_orientation),
        js_string!("setTabOrientation"),
        1,
    );
    let chrome_obj = chrome_builder.build();

    let mut rix_builder = ObjectInitializer::new(context);
    rix_builder.property(
        js_string!("tabs"),
        tabs_obj,
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    rix_builder.property(
        js_string!("navigation"),
        nav_obj,
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    rix_builder.property(
        js_string!("chrome"),
        chrome_obj,
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    rix_builder.function(
        NativeFunction::from_fn_ptr(rix_post_message),
        js_string!("postMessage"),
        1,
    );

    let rix_obj = rix_builder.build();

    context.register_global_property(
        js_string!("rix"),
        rix_obj.clone(),
        Attribute::READONLY | Attribute::CONFIGURABLE,
    )?;

    let script = r#"
        (function(rixObj) {
            if (typeof window === "undefined") {
                globalThis.window = globalThis;
            }
            window.rix = rixObj;
        })
    "#;
    let _ = context
        .eval(boa_engine::Source::from_bytes(script.as_bytes()))
        .and_then(|func| {
            if let Some(f) = func.as_callable() {
                f.call(&JsValue::undefined(), &[rix_obj.into()], context)
            } else {
                Ok(JsValue::undefined())
            }
        });

    Ok(true)
}

fn rix_tabs_create(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let mut target_url = "about:blank".to_string();
    if let Some(arg) = args.first() {
        if let Some(obj) = arg.as_object() {
            if let Ok(url_val) = obj.get(js_string!("url"), context) {
                if let Ok(js_str) = url_val.to_string(context) {
                    target_url = js_str.to_std_string_escaped();
                }
            }
        } else if let Ok(js_str) = arg.to_string(context) {
            target_url = js_str.to_std_string_escaped();
        }
    }

    let state = context
        .get_data::<Rc<RefCell<PrivilegedHostState>>>()
        .cloned();
    if let Some(state) = state {
        let new_id = state.borrow_mut().create_tab(target_url);
        return Ok(JsValue::new(new_id));
    }
    Ok(JsValue::new(0))
}

fn rix_tabs_close(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let tab_id = args.get_or_undefined(0).to_number(context).unwrap_or(0.0) as u32;
    let state = context
        .get_data::<Rc<RefCell<PrivilegedHostState>>>()
        .cloned();
    if let Some(state) = state {
        let ok = state.borrow_mut().close_tab(tab_id);
        return Ok(JsValue::new(ok));
    }
    Ok(JsValue::new(false))
}

fn rix_tabs_switch(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let tab_id = args.get_or_undefined(0).to_number(context).unwrap_or(0.0) as u32;
    let state = context
        .get_data::<Rc<RefCell<PrivilegedHostState>>>()
        .cloned();
    if let Some(state) = state {
        let ok = state.borrow_mut().switch_tab(tab_id);
        return Ok(JsValue::new(ok));
    }
    Ok(JsValue::new(false))
}

fn rix_tabs_list(_this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let state = context
        .get_data::<Rc<RefCell<PrivilegedHostState>>>()
        .cloned();
    let mut elements = Vec::new();
    if let Some(state) = state {
        let tabs = state.borrow().tabs.clone();
        for tab in tabs {
            let mut tab_builder = ObjectInitializer::new(context);
            tab_builder.property(
                js_string!("id"),
                JsValue::new(tab.id),
                Attribute::READONLY | Attribute::ENUMERABLE,
            );
            tab_builder.property(
                js_string!("title"),
                js_string!(tab.title),
                Attribute::READONLY | Attribute::ENUMERABLE,
            );
            tab_builder.property(
                js_string!("url"),
                js_string!(tab.url),
                Attribute::READONLY | Attribute::ENUMERABLE,
            );
            tab_builder.property(
                js_string!("isActive"),
                JsValue::new(tab.is_active),
                Attribute::READONLY | Attribute::ENUMERABLE,
            );
            elements.push(tab_builder.build().into());
        }
    }
    let array = boa_engine::object::builtins::JsArray::from_iter(elements, context);
    Ok(array.into())
}

fn rix_navigation_go_to(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let url = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let state = context
        .get_data::<Rc<RefCell<PrivilegedHostState>>>()
        .cloned();
    if let Some(state) = state {
        state
            .borrow_mut()
            .navigation_history
            .push(NavigationCommand::GoTo(url));
    }
    Ok(JsValue::undefined())
}

fn rix_navigation_back(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = context
        .get_data::<Rc<RefCell<PrivilegedHostState>>>()
        .cloned();
    if let Some(state) = state {
        state
            .borrow_mut()
            .navigation_history
            .push(NavigationCommand::Back);
    }
    Ok(JsValue::undefined())
}

fn rix_navigation_forward(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = context
        .get_data::<Rc<RefCell<PrivilegedHostState>>>()
        .cloned();
    if let Some(state) = state {
        state
            .borrow_mut()
            .navigation_history
            .push(NavigationCommand::Forward);
    }
    Ok(JsValue::undefined())
}

fn rix_navigation_reload(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = context
        .get_data::<Rc<RefCell<PrivilegedHostState>>>()
        .cloned();
    if let Some(state) = state {
        state
            .borrow_mut()
            .navigation_history
            .push(NavigationCommand::Reload);
    }
    Ok(JsValue::undefined())
}

fn rix_chrome_set_theme(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let mode = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    if mode != "dark" && mode != "light" {
        return Ok(JsValue::new(false));
    }
    let state = context
        .get_data::<Rc<RefCell<PrivilegedHostState>>>()
        .cloned();
    if let Some(state) = state {
        state.borrow_mut().requested_theme = Some(mode);
        return Ok(JsValue::new(true));
    }
    Ok(JsValue::new(false))
}

fn rix_chrome_set_tab_orientation(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let orientation = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    if orientation != "horizontal" && orientation != "vertical" {
        return Ok(JsValue::new(false));
    }
    let state = context
        .get_data::<Rc<RefCell<PrivilegedHostState>>>()
        .cloned();
    if let Some(state) = state {
        state.borrow_mut().requested_tab_orientation = Some(orientation);
        return Ok(JsValue::new(true));
    }
    Ok(JsValue::new(false))
}

fn rix_post_message(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let msg = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let state = context
        .get_data::<Rc<RefCell<PrivilegedHostState>>>()
        .cloned();
    if let Some(state) = state {
        state.borrow_mut().posted_messages.push(msg);
    }
    Ok(JsValue::undefined())
}
