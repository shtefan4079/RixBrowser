
use super::*;
use boa_engine::{
    builtins::promise::PromiseState,
    module::Module,
    Context, JsResult, Source,
};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

pub fn is_classic_script_type(raw: &str) -> bool {
    let essence = raw
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    essence.is_empty()
        || matches!(
            essence.as_str(),
            "text/javascript"
                | "application/javascript"
                | "text/ecmascript"
                | "application/ecmascript"
                | "text/jscript"
                | "text/livescript"
                | "text/javascript1.0"
                | "text/javascript1.1"
                | "text/javascript1.2"
                | "text/javascript1.3"
                | "text/javascript1.4"
                | "text/javascript1.5"
        )
}

pub fn is_module_script_type(raw: &str) -> bool {
    raw.trim().eq_ignore_ascii_case("module")
}

pub(super) struct CurrentScriptGuard {
    state: Rc<RefCell<DomBridgeState>>,
    previous: Option<NodeId>,
}

impl CurrentScriptGuard {
    pub(super) fn enter(state: &Rc<RefCell<DomBridgeState>>, current: NodeId) -> Self {
        let previous = {
            let mut s = state.borrow_mut();
            let prev = s.current_script;
            s.current_script = Some(current);
            prev
        };
        Self {
            state: state.clone(),
            previous,
        }
    }
}

impl Drop for CurrentScriptGuard {
    fn drop(&mut self) {
        self.state.borrow_mut().current_script = self.previous;
    }
}

pub(super) fn node_document_root(node: NodeId, document: &Document) -> Option<NodeId> {
    let mut current = node;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        if matches!(document.get(current), Some(NodeData::Document)) {
            return Some(current);
        }
        if let Some(parent) = document.parent(current) {
            current = parent;
            continue;
        }
        match document.get(current) {
            Some(NodeData::ShadowRoot { host, .. }) => current = *host,
            _ => return None,
        }
    }
    None
}

pub(super) fn is_connected_to_browsing_context(node: NodeId, state: &DomBridgeState) -> bool {
    let doc = state.document.borrow();
    if let Some(root) = node_document_root(node, &doc) {
        !state.inert_documents.contains(&root)
    } else {
        false
    }
}

pub(super) fn prepare_connected_scripts(nodes: &[NodeId], context: &mut Context) -> JsResult<()> {
    let state = match context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        Some(s) => s,
        None => return Ok(()),
    };

    let mut scripts_to_prepare = Vec::new();
    {
        let bridge = state.borrow();
        let doc = bridge.document.borrow();
        for &root in nodes {
            let Some(NodeData::Element(element)) = doc.get(root) else {
                continue;
            };
            if doc.children(root).next().is_none() {
                if element.name.ns.as_str() == "http://www.w3.org/1999/xhtml"
                    && element.name.local.as_str() == "script"
                    && !bridge.is_script_started(root)
                    && is_connected_to_browsing_context(root, &bridge)
                {
                    scripts_to_prepare.push(root);
                }
                continue;
            }
            for node in doc.descendants(root) {
                if bridge.is_script_started(node) {
                    continue;
                }
                if let Some(NodeData::Element(element)) = doc.get(node) {
                    if element.name.ns.as_str() == "http://www.w3.org/1999/xhtml"
                        && element.name.local.as_str() == "script"
                    {
                        if is_connected_to_browsing_context(node, &bridge) {
                            scripts_to_prepare.push(node);
                        }
                    }
                }
            }
        }
    }

    for script_node in scripts_to_prepare {
        prepare_script_element(script_node, &state, context)?;
    }

    Ok(())
}

fn prepare_script_element(
    node: NodeId,
    state: &Rc<RefCell<DomBridgeState>>,
    context: &mut Context,
) -> JsResult<()> {
    if state.borrow().is_script_started(node) {
        return Ok(());
    }

    if !is_connected_to_browsing_context(node, &state.borrow()) {
        return Ok(());
    }

    let (type_attr, src_attr, source_code) = {
        let bridge = state.borrow();
        let doc = bridge.document.borrow();
        let Some(NodeData::Element(element)) = doc.get(node) else {
            return Ok(());
        };
        let type_attr = element.get_attribute("type").map(str::to_string);
        let src_attr = element.get_attribute("src").map(str::to_string);
        let source_code = doc.inner_text(node);
        (type_attr, src_attr, source_code)
    };

    let type_str = type_attr.as_deref().unwrap_or("");
    let is_classic = is_classic_script_type(type_str);
    let is_module = is_module_script_type(type_str);

    if !is_classic && !is_module {
        state.borrow_mut().mark_script_started(node);
        return Ok(());
    }

    if let Some(src) = src_attr {
        if src.trim().is_empty() {
            state.borrow_mut().mark_script_started(node);
            let properties = [
                ("bubbles", boa_engine::JsValue::new(false)),
                ("cancelable", boa_engine::JsValue::new(false)),
            ];
            super::events::dispatch_dom_event_to_js(node, "error", Some(&properties), context)?;
        } else {
        }
        return Ok(());
    }

    if source_code.is_empty() {
        return Ok(());
    }

    state.borrow_mut().mark_script_started(node);

    if is_classic {
        let _guard = CurrentScriptGuard::enter(state, node);
        let win_obj = {
            let bridge = state.borrow();
            let doc = bridge.document.borrow();
            if let Some(root) = node_document_root(node, &doc) {
                bridge.document_default_views.get(&root).cloned()
            } else {
                None
            }
        };
        if let Some(ref win) = win_obj {
            if let Ok(push_val) = context.global_object().get(js_string!("__rixPushActiveWindow"), context) {
                if let Some(push_fn) = push_val.as_object() {
                    let _ = push_fn.call(&JsValue::undefined(), &[win.clone().into()], context);
                }
            }
        }
        let source = Source::from_bytes(source_code.as_bytes());
        let eval_res = context.eval(source);
        if win_obj.is_some() {
            if let Ok(pop_val) = context.global_object().get(js_string!("__rixPopActiveWindow"), context) {
                if let Some(pop_fn) = pop_val.as_object() {
                    let _ = pop_fn.call(&JsValue::undefined(), &[], context);
                }
            }
        }
        if let Err(error) = eval_res {
            crate::exceptions::report(error, context);
        }
        let _ = context.run_jobs();
    } else if is_module {
        let win_obj = {
            let bridge = state.borrow();
            let doc = bridge.document.borrow();
            if let Some(root) = node_document_root(node, &doc) {
                bridge.document_default_views.get(&root).cloned()
            } else {
                None
            }
        };
        if let Some(ref win) = win_obj {
            if let Ok(push_val) = context.global_object().get(js_string!("__rixPushActiveWindow"), context) {
                if let Some(push_fn) = push_val.as_object() {
                    let _ = push_fn.call(&JsValue::undefined(), &[win.clone().into()], context);
                }
            }
        }
        let source = Source::from_bytes(source_code.as_bytes());
        let parse_res = Module::parse(source, None, context);
        match parse_res {
            Ok(module) => {
                let loaded = module.load(context);
                let _ = context.run_jobs();
                if matches!(loaded.state(), PromiseState::Fulfilled(_)) {
                    if let Err(error) = module.link(context) {
                        crate::exceptions::report(error, context);
                    } else {
                        match module.evaluate(context) {
                            Ok(evaluated) => {
                                let _ = context.run_jobs();
                                if let PromiseState::Rejected(reason) = evaluated.state() {
                                    crate::exceptions::report_message(
                                        format!("uncaught (in promise) {reason:?}"),
                                        context,
                                    );
                                }
                            }
                            Err(error) => {
                                crate::exceptions::report(error, context);
                            }
                        }
                    }
                } else if let PromiseState::Rejected(reason) = loaded.state() {
                    crate::exceptions::report_message(
                        format!("uncaught (in promise) {reason:?}"),
                        context,
                    );
                }
            }
            Err(error) => {
                crate::exceptions::report(error, context);
            }
        }
        if win_obj.is_some() {
            if let Ok(pop_val) = context.global_object().get(js_string!("__rixPopActiveWindow"), context) {
                if let Some(pop_fn) = pop_val.as_object() {
                    let _ = pop_fn.call(&JsValue::undefined(), &[], context);
                }
            }
        }
    }

    Ok(())
}
