
use boa_engine::{
    js_string, native_function::NativeFunction, object::ObjectInitializer, property::Attribute,
    Context, JsError, JsResult, JsValue, Script, Source,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleLogLevel {
    Log,
    Warn,
    Error,
    Info,
    Debug,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleMessage {
    pub level: ConsoleLogLevel,
    pub message: String,
    pub timestamp_millis: u64,
}

#[derive(Debug, Clone)]
pub struct JsRuntimeConfig {
    pub loop_iteration_limit: u64,
    pub recursion_limit: usize,
    pub stack_size: usize,
    pub backtrace_limit: usize,
    pub jobs_per_checkpoint: usize,
    pub job_time_slice: std::time::Duration,
    pub origin: Option<String>,
    pub code_cache: Option<std::sync::Arc<crate::code_cache::CodeCache>>,
}

impl Default for JsRuntimeConfig {
    fn default() -> Self {
        Self {
            loop_iteration_limit: 200_000,
            recursion_limit: 512,
            stack_size: 1024 * 10,
            backtrace_limit: 50,
            jobs_per_checkpoint: 256,
            job_time_slice: std::time::Duration::from_millis(4),
            origin: None,
            code_cache: None,
        }
    }
}

impl JsRuntimeConfig {
    pub fn for_tab_origin(origin: impl Into<String>) -> Self {
        Self {
            origin: Some(origin.into()),
            ..Self::default()
        }
    }

    pub fn for_page_worker(origin: impl Into<String>) -> Self {
        Self {
            origin: Some(origin.into()),
            loop_iteration_limit: u64::MAX,
            recursion_limit: 4_096,
            stack_size: 1024 * 128,
            jobs_per_checkpoint: 16_384,
            job_time_slice: std::time::Duration::from_millis(100),
            ..Self::default()
        }
    }

    pub fn with_code_cache(
        mut self,
        code_cache: std::sync::Arc<crate::code_cache::CodeCache>,
    ) -> Self {
        self.code_cache = Some(code_cache);
        self
    }
}

#[derive(Error, Debug, Clone, PartialEq)]
pub enum JsRuntimeError {
    #[error("JavaScript Execution Error: {0}")]
    EvaluationError(String),

    #[error("JavaScript Timeout: execution exceeded {0} ms")]
    Timeout(u64),

    #[error("JavaScript Limit Exceeded: {0}")]
    LimitExceeded(String),
}

#[derive(Default)]
pub struct RuntimeHostState {
    pub console_messages: Vec<ConsoleMessage>,
    console_counters: std::collections::HashMap<String, u32>,
    console_timers: std::collections::HashMap<String, std::time::Instant>,
}

pub struct JsRuntime {
    context: Context,
    host_state: Rc<RefCell<RuntimeHostState>>,
    config: JsRuntimeConfig,
    code_cache: std::sync::Arc<crate::code_cache::CodeCache>,
    script_cache: RefCell<HashMap<u64, Script>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScriptEvaluationTiming {
    pub parse_ms: f64,
    pub compile_ms: f64,
    pub execute_ms: f64,
    pub total_ms: f64,
}

impl JsRuntime {
    pub fn new(config: JsRuntimeConfig) -> Self {
        Self::new_with_module_loader(config, Rc::new(boa_engine::module::IdleModuleLoader))
    }

    pub fn new_with_module_loader<M>(config: JsRuntimeConfig, module_loader: Rc<M>) -> Self
    where
        M: boa_engine::module::ModuleLoader + 'static,
    {
        let jobs = Rc::new(crate::jobs::BrowserJobExecutor::new(
            config.jobs_per_checkpoint,
            config.job_time_slice,
        ));
        let promise_rejections =
            Rc::new(RefCell::new(crate::jobs::PromiseRejectionTracker::default()));
        let mut context = Context::builder()
            .job_executor(jobs.clone())
            .host_hooks(Rc::new(crate::jobs::PromiseRejectionHooks::new(
                promise_rejections.clone(),
            )))
            .module_loader(module_loader)
            .build()
            .expect("default Boa context must build");
        context.insert_data(jobs);
        context.insert_data(promise_rejections);

        let mut limits = context.runtime_limits();
        limits.set_loop_iteration_limit(config.loop_iteration_limit);
        limits.set_recursion_limit(config.recursion_limit);
        limits.set_stack_size_limit(config.stack_size);
        limits.set_backtrace_limit(config.backtrace_limit);
        context.set_runtime_limits(limits);

        let host_state = Rc::new(RefCell::new(RuntimeHostState::default()));
        context.insert_data(host_state.clone());

        let code_cache = config
            .code_cache
            .clone()
            .unwrap_or_else(crate::code_cache::default_code_cache);

        let mut runtime = Self {
            context,
            host_state,
            config,
            code_cache,
            script_cache: RefCell::new(HashMap::new()),
        };

        runtime.register_console();
        runtime
    }

    pub fn new_for_tab(origin: impl Into<String>) -> Self {
        Self::new(JsRuntimeConfig::for_tab_origin(origin))
    }

    pub fn code_cache(&self) -> &std::sync::Arc<crate::code_cache::CodeCache> {
        &self.code_cache
    }

    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }

    pub fn context(&self) -> &Context {
        &self.context
    }

    pub fn host_state(&self) -> Rc<RefCell<RuntimeHostState>> {
        self.host_state.clone()
    }

    pub fn config(&self) -> &JsRuntimeConfig {
        &self.config
    }

    pub fn evaluate(&mut self, source: &str) -> Result<JsValue, JsRuntimeError> {
        self.context
            .eval(Source::from_bytes(source.as_bytes()))
            .map_err(|err| self.format_error(err))
    }

    pub fn evaluate_with_phase_timing(
        &mut self,
        source: &str,
    ) -> (Result<JsValue, JsRuntimeError>, ScriptEvaluationTiming) {
        self.evaluate_with_phase_timing_at_source(source, None)
    }

    pub fn evaluate_with_phase_timing_at_path(
        &mut self,
        source: &str,
        path: &Path,
    ) -> (Result<JsValue, JsRuntimeError>, ScriptEvaluationTiming) {
        self.evaluate_with_phase_timing_at_source(source, Some(path))
    }

    pub fn evaluate_with_phase_callback<F>(
        &mut self,
        source: &str,
        source_path: Option<&Path>,
        mut on_phase: F,
    ) -> (Result<JsValue, JsRuntimeError>, ScriptEvaluationTiming)
    where
        F: FnMut(&'static str),
    {
        let total_started = std::time::Instant::now();
        let mut timing = ScriptEvaluationTiming::default();

        let cache_key = crate::code_cache::CodeCache::hash_source(source, source_path);

        on_phase("parse");
        let parse_started = std::time::Instant::now();
        let cached = self.script_cache.borrow().get(&cache_key).cloned();
        let parsed = if let Some(script) = cached {
            timing.parse_ms = 0.0;
            Ok(script)
        } else {
            let parsed_result = self
                .code_cache
                .load_or_parse(source, source_path, &mut self.context);
            let parse_elapsed = parse_started.elapsed().as_secs_f64() * 1000.0;
            match parsed_result {
                Ok((script, _from_cache)) => {
                    self.script_cache
                        .borrow_mut()
                        .insert(cache_key, script.clone());
                    timing.parse_ms = parse_elapsed;
                    Ok(script)
                }
                Err(error) => {
                    timing.parse_ms = parse_elapsed;
                    Err(error)
                }
            }
        };

        let result = match parsed {
            Err(error) => Err(self.format_error(error)),
            Ok(script) => {
                on_phase("compile");
                let compile_started = std::time::Instant::now();
                let compiled = script.codeblock(&mut self.context);
                timing.compile_ms = compile_started.elapsed().as_secs_f64() * 1000.0;
                match compiled {
                    Err(error) => Err(self.format_error(error)),
                    Ok(_) => {
                        on_phase("execute");
                        let execute_started = std::time::Instant::now();
                        let evaluated = script.evaluate(&mut self.context);
                        timing.execute_ms = execute_started.elapsed().as_secs_f64() * 1000.0;
                        evaluated.map_err(|error| self.format_error(error))
                    }
                }
            }
        };

        timing.total_ms = total_started.elapsed().as_secs_f64() * 1000.0;
        (result, timing)
    }

    fn evaluate_with_phase_timing_at_source(
        &mut self,
        source: &str,
        source_path: Option<&Path>,
    ) -> (Result<JsValue, JsRuntimeError>, ScriptEvaluationTiming) {
        self.evaluate_with_phase_callback(source, source_path, |_| {})
    }

    pub fn evaluate_with_timeout(
        &mut self,
        source: &str,
        timeout_ms: u64,
    ) -> Result<JsValue, JsRuntimeError> {
        let start = std::time::Instant::now();
        let res = self.evaluate(source);
        if start.elapsed().as_millis() as u64 > timeout_ms {
            return Err(JsRuntimeError::Timeout(timeout_ms));
        }
        res
    }

    pub fn run_jobs(&mut self) {
        let _ = self.context.run_jobs();
    }

    pub fn console_messages(&self) -> Vec<ConsoleMessage> {
        self.host_state.borrow().console_messages.clone()
    }

    pub fn take_console_messages(&mut self) -> Vec<ConsoleMessage> {
        std::mem::take(&mut self.host_state.borrow_mut().console_messages)
    }

    pub fn clear_console(&self) {
        self.host_state.borrow_mut().console_messages.clear();
    }

    fn format_error(&mut self, error: JsError) -> JsRuntimeError {
        let msg = format!("{error}");
        let lower = msg.to_lowercase();
        if lower.contains("runtimelimit")
            || lower.contains("loop")
            || lower.contains("iteration")
            || lower.contains("recursion")
            || lower.contains("maximum call stack")
            || lower.contains("stack size")
            || lower.contains("stack overflow")
            || lower.contains("limit exceeded")
        {
            JsRuntimeError::LimitExceeded(msg)
        } else {
            JsRuntimeError::EvaluationError(msg)
        }
    }

    fn register_console(&mut self) {
        let mut console_obj = ObjectInitializer::new(&mut self.context);

        console_obj.function(
            NativeFunction::from_fn_ptr(console_log_fn),
            js_string!("log"),
            1,
        );

        console_obj.function(
            NativeFunction::from_fn_ptr(console_warn_fn),
            js_string!("warn"),
            1,
        );

        console_obj.function(
            NativeFunction::from_fn_ptr(console_error_fn),
            js_string!("error"),
            1,
        );

        console_obj.function(
            NativeFunction::from_fn_ptr(console_info_fn),
            js_string!("info"),
            1,
        );

        console_obj.function(
            NativeFunction::from_fn_ptr(console_debug_fn),
            js_string!("debug"),
            1,
        );

        console_obj.function(
            NativeFunction::from_fn_ptr(console_clear_fn),
            js_string!("clear"),
            0,
        );

        console_obj.function(
            NativeFunction::from_fn_ptr(console_assert_fn),
            js_string!("assert"),
            1,
        );

        console_obj.function(
            NativeFunction::from_fn_ptr(console_count_fn),
            js_string!("count"),
            1,
        );
        console_obj.function(
            NativeFunction::from_fn_ptr(console_count_reset_fn),
            js_string!("countReset"),
            1,
        );

        console_obj.function(
            NativeFunction::from_fn_ptr(console_time_fn),
            js_string!("time"),
            1,
        );
        console_obj.function(
            NativeFunction::from_fn_ptr(console_time_end_fn),
            js_string!("timeEnd"),
            1,
        );
        console_obj.function(
            NativeFunction::from_fn_ptr(console_time_log_fn),
            js_string!("timeLog"),
            1,
        );

        let console = console_obj.build();

        self.context
            .register_global_property(
                js_string!("console"),
                console,
                Attribute::WRITABLE | Attribute::CONFIGURABLE,
            )
            .expect("Registering global console object");
    }
}

fn format_console_arg(arg: &JsValue, context: &mut Context) -> String {
    if arg.is_null() {
        return "null".to_string();
    }
    if arg.is_undefined() {
        return "undefined".to_string();
    }
    if let Some(b) = arg.as_boolean() {
        return b.to_string();
    }
    if let Some(n) = arg.as_number() {
        if n.fract() == 0.0 && n.is_finite() && n.abs() < 1e21 {
            return format!("{}", n as i64);
        }
        return n.to_string();
    }
    if let Some(s) = arg.as_string() {
        return s.to_std_string_escaped();
    }
    if arg.is_object() {
        let json_probe = context.eval(Source::from_bytes(
            b"typeof JSON !== 'undefined' && typeof JSON.stringify === 'function'",
        ));
        let has_json = json_probe
            .ok()
            .and_then(|v| v.as_boolean())
            .unwrap_or(false);
        if has_json {
            let global = context.global_object();
            let stored = global.set(js_string!("__rix_console_tmp"), arg.clone(), false, context);
            if stored.is_ok() {
                let res = context.eval(Source::from_bytes(
                    b"JSON.stringify(globalThis.__rix_console_tmp)",
                ));
                let _ = global.delete_property_or_throw(js_string!("__rix_console_tmp"), context);
                if let Ok(v) = res {
                    if let Some(s) = v.as_string() {
                        return s.to_std_string_escaped();
                    }
                }
            }
        }
    }
    if let Ok(js_str) = arg.to_string(context) {
        js_str.to_std_string_escaped()
    } else {
        "[Unknown Value]".to_string()
    }
}

fn push_console_message(level: ConsoleLogLevel, message: String, context: &mut Context) {
    let state = context.get_data::<Rc<RefCell<RuntimeHostState>>>().cloned();
    let timestamp_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if let Some(state) = state {
        state.borrow_mut().console_messages.push(ConsoleMessage {
            level,
            message,
            timestamp_millis,
        });
    }
}

fn append_console_log(level: ConsoleLogLevel, args: &[JsValue], context: &mut Context) {
    let mut parts = Vec::with_capacity(args.len());
    for arg in args {
        parts.push(format_console_arg(arg, context));
    }
    push_console_message(level, parts.join(" "), context);
}

fn console_log_fn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    append_console_log(ConsoleLogLevel::Log, args, context);
    Ok(JsValue::undefined())
}

fn console_warn_fn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    append_console_log(ConsoleLogLevel::Warn, args, context);
    Ok(JsValue::undefined())
}

fn console_error_fn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    append_console_log(ConsoleLogLevel::Error, args, context);
    Ok(JsValue::undefined())
}

fn console_info_fn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    append_console_log(ConsoleLogLevel::Info, args, context);
    Ok(JsValue::undefined())
}

fn console_debug_fn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    append_console_log(ConsoleLogLevel::Debug, args, context);
    Ok(JsValue::undefined())
}

fn console_clear_fn(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Some(state) = context.get_data::<Rc<RefCell<RuntimeHostState>>>().cloned() {
        state.borrow_mut().console_messages.clear();
    }
    Ok(JsValue::undefined())
}

fn console_assert_fn(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let cond = args.get_or_undefined(0).to_boolean();
    if !cond {
        let rest = if args.len() > 1 { &args[1..] } else { &[] };
        let mut parts = vec!["Assertion failed".to_string()];
        for a in rest {
            parts.push(format_console_arg(a, context));
        }
        let msg = if rest.is_empty() {
            parts[0].clone()
        } else {
            format!("{}: {}", parts[0], parts[1..].join(" "))
        };
        push_console_message(ConsoleLogLevel::Error, msg, context);
    }
    Ok(JsValue::undefined())
}

fn console_count_fn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let label = args
        .get_or_undefined(0)
        .to_string(context)
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_else(|_| "default".to_string());
    let label = if label.is_empty() || label == "undefined" {
        "default".to_string()
    } else {
        label
    };
    let count = if let Some(state) = context.get_data::<Rc<RefCell<RuntimeHostState>>>().cloned() {
        let mut s = state.borrow_mut();
        let c = s.console_counters.entry(label.clone()).or_insert(0);
        *c += 1;
        *c
    } else {
        1
    };
    push_console_message(ConsoleLogLevel::Log, format!("{label}: {count}"), context);
    Ok(JsValue::undefined())
}

fn console_count_reset_fn(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let label = args
        .get_or_undefined(0)
        .to_string(context)
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_else(|_| "default".to_string());
    if let Some(state) = context.get_data::<Rc<RefCell<RuntimeHostState>>>().cloned() {
        state.borrow_mut().console_counters.remove(&label);
    }
    Ok(JsValue::undefined())
}

fn console_time_fn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let label = args
        .get_or_undefined(0)
        .to_string(context)
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_else(|_| "default".to_string());
    if let Some(state) = context.get_data::<Rc<RefCell<RuntimeHostState>>>().cloned() {
        state
            .borrow_mut()
            .console_timers
            .insert(label, std::time::Instant::now());
    }
    Ok(JsValue::undefined())
}

fn console_time_end_fn(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let label = args
        .get_or_undefined(0)
        .to_string(context)
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_else(|_| "default".to_string());
    let elapsed = if let Some(state) = context.get_data::<Rc<RefCell<RuntimeHostState>>>().cloned()
    {
        state.borrow_mut().console_timers.remove(&label)
    } else {
        None
    };
    match elapsed {
        Some(start) => push_console_message(
            ConsoleLogLevel::Log,
            format!("{label}: {:.3}ms", start.elapsed().as_secs_f64() * 1000.0),
            context,
        ),
        None => push_console_message(
            ConsoleLogLevel::Warn,
            format!("Timer '{label}' does not exist"),
            context,
        ),
    }
    Ok(JsValue::undefined())
}

fn console_time_log_fn(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    use boa_engine::JsArgs;
    let label = args
        .get_or_undefined(0)
        .to_string(context)
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_else(|_| "default".to_string());
    let elapsed = if let Some(state) = context.get_data::<Rc<RefCell<RuntimeHostState>>>().cloned()
    {
        state.borrow().console_timers.get(&label).copied()
    } else {
        None
    };
    match elapsed {
        Some(start) => {
            let mut parts = vec![format!(
                "{label}: {:.3}ms",
                start.elapsed().as_secs_f64() * 1000.0
            )];
            for extra in args.iter().skip(1) {
                parts.push(format_console_arg(extra, context));
            }
            push_console_message(ConsoleLogLevel::Log, parts.join(" "), context);
        }
        None => push_console_message(
            ConsoleLogLevel::Warn,
            format!("Timer '{label}' does not exist"),
            context,
        ),
    }
    Ok(JsValue::undefined())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_phase_timing_and_caching() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache = std::sync::Arc::new(crate::code_cache::CodeCache::new(Some(
            temp_dir.path().to_path_buf(),
        )));

        let config1 = JsRuntimeConfig::default().with_code_cache(cache.clone());
        let mut runtime1 = JsRuntime::new(config1);

        let code = r#"
            let acc = 0;
            for (let i = 1; i <= 10; i++) {
                acc += i;
            }
            acc;
        "#;

        let (val1, timing1) = runtime1.evaluate_with_phase_timing(code);
        assert_eq!(val1.unwrap().as_number().unwrap(), 55.0);
        assert!(timing1.total_ms > 0.0);

        let (val2, timing2) = runtime1.evaluate_with_phase_timing(code);
        assert_eq!(val2.unwrap().as_number().unwrap(), 55.0);
        assert_eq!(timing2.parse_ms, 0.0);

        let config2 = JsRuntimeConfig::default().with_code_cache(cache);
        let mut runtime2 = JsRuntime::new(config2);
        let (val3, _timing3) = runtime2.evaluate_with_phase_timing(code);
        assert_eq!(val3.unwrap().as_number().unwrap(), 55.0);
    }

    #[test]
    fn test_direct_codeblock_sharing_between_contexts() {
        let code = r#"
            let factor = 3;
            const offset = 2;
            function multiply(x, y) {
                return x * y * factor + offset;
            }
            var result = multiply(6, 7);
            result;
        "#;

        let codeblock = {
            let mut context1 = Context::default();
            let source = Source::from_bytes(code.as_bytes());
            let script1 = Script::parse(source, None, &mut context1).unwrap();
            script1.codeblock(&mut context1).unwrap()
        };

        {
            let mut context2 = Context::default();
            let script2 = Script::from_codeblock(codeblock, Some(code), None, None, &mut context2);
            let val = script2.evaluate(&mut context2).unwrap();
            assert_eq!(val.as_number().unwrap(), 128.0);
        }
    }

    #[test]
    fn test_hot_jit_execution_with_comparisons_and_try_catch() {
        let mut runtime = JsRuntime::new(JsRuntimeConfig::default());

        let code = r#"
            function compute(x, y) {
                let r = 0;
                if (x === y) {
                    r = 100;
                } else if (x < y) {
                    r = y - x;
                } else {
                    r = x - y;
                }
                return r;
            }

            let total = 0;
            for (let i = 0; i < 50; i++) {
                total += compute(i, 25);
            }

            function safeDivide(a, b) {
                try {
                    if (b === 0) {
                        throw new Error("div0");
                    }
                    return a / b;
                } catch (e) {
                    return -1;
                }
            }

            for (let i = 0; i < 30; i++) {
                total += safeDivide(i, 2);
            }
            total += safeDivide(10, 0);

            let loopSum = 0;
            for (let k = 0; k < 200; k++) {
                if ((k & 1) === 0) {
                    loopSum += k;
                }
            }

            total + loopSum;
        "#;

        let (val, timing) = runtime.evaluate_with_phase_timing(code);
        assert!(val.is_ok(), "evaluation succeeded: {:?}", val);
        let num = val.unwrap().as_number().unwrap();
        assert!(num > 0.0, "total was {}", num);
        assert!(timing.execute_ms > 0.0);
    }

    #[test]
    fn test_dom_lazy_accessors_and_custom_element_upgrade() {
        use crate::dom_bridge::{register_dom_bridge, DomBridgeState};
        use rix_dom::Document;
        use std::cell::RefCell;
        use std::rc::Rc;

        let mut runtime = JsRuntime::new(JsRuntimeConfig::default());
        let doc = Rc::new(RefCell::new(Document::new()));
        let dom_state = Rc::new(RefCell::new(DomBridgeState::with_shared_document(doc)));
        register_dom_bridge(runtime.context_mut(), dom_state).expect("dom bridge registered");

        let code = r#"
            const div = document.createElement('div');
            div.dataset.testProp = 'hello';
            const dsVal = div.dataset.testProp;
            const hasDataAttr = div.getAttribute('data-test-prop') === 'hello';

            div.style.color = 'red';
            const styleVal = div.style.color;

            div.classList.add('c1', 'c2');
            const hasC1 = div.classList.contains('c1');
            const classAttr = div.getAttribute('class');
            const sameList = div.classList === div.classList;

            class MyElement extends HTMLElement {
                constructor() {
                    super();
                    this.upgraded = true;
                }
            }
            const preEl = document.createElement('my-element');
            document.appendChild(preEl);
            customElements.define('my-element', MyElement);
            const postEl = document.createElement('my-element');

            class ShadowHost extends HTMLElement {
                constructor() {
                    super();
                    this.attachShadow({ mode: 'open' });
                }
            }
            customElements.define('shadow-host', ShadowHost);
            const host = document.createElement('shadow-host');
            document.appendChild(host);

            const nestedEl = document.createElement('nested-element');
            host.shadowRoot.appendChild(nestedEl);

            let nestedConnected = false;
            class NestedElement extends HTMLElement {
                constructor() {
                    super();
                    this.nestedUpgraded = true;
                }
                connectedCallback() {
                    nestedConnected = true;
                }
            }
            customElements.define('nested-element', NestedElement);

            ({
                dsVal,
                hasDataAttr,
                styleVal,
                hasC1,
                classAttr,
                sameList,
                preUpgraded: preEl.upgraded === true,
                postUpgraded: postEl.upgraded === true,
                nestedUpgraded: nestedEl.nestedUpgraded === true,
                nestedConnected,
            })
        "#;
        let res = runtime.evaluate(code).expect("script evaluation");
        let obj = res.as_object().expect("returned object");
        let has_c1 = obj
            .get(boa_engine::js_string!("hasC1"), runtime.context_mut())
            .unwrap();
        assert!(has_c1.to_boolean());
        let class_attr = obj
            .get(boa_engine::js_string!("classAttr"), runtime.context_mut())
            .unwrap();
        assert_eq!(
            class_attr
                .to_string(runtime.context_mut())
                .unwrap()
                .to_std_string_escaped(),
            "c1 c2"
        );
        let same_list = obj
            .get(boa_engine::js_string!("sameList"), runtime.context_mut())
            .unwrap();
        assert!(same_list.to_boolean());
        let ds = obj
            .get(boa_engine::js_string!("dsVal"), runtime.context_mut())
            .unwrap();
        assert_eq!(
            ds.to_string(runtime.context_mut())
                .unwrap()
                .to_std_string_escaped(),
            "hello"
        );
        let has_attr = obj
            .get(boa_engine::js_string!("hasDataAttr"), runtime.context_mut())
            .unwrap();
        assert!(has_attr.to_boolean());
        let pre_up = obj
            .get(boa_engine::js_string!("preUpgraded"), runtime.context_mut())
            .unwrap();
        assert!(pre_up.to_boolean());
        let post_up = obj
            .get(boa_engine::js_string!("postUpgraded"), runtime.context_mut())
            .unwrap();
        assert!(post_up.to_boolean());
        let nested_up = obj
            .get(boa_engine::js_string!("nestedUpgraded"), runtime.context_mut())
            .unwrap();
        assert!(nested_up.to_boolean());
        let nested_conn = obj
            .get(boa_engine::js_string!("nestedConnected"), runtime.context_mut())
            .unwrap();
        assert!(nested_conn.to_boolean());
    }

    #[test]
    fn test_computed_style_fast_path() {
        use crate::dom_bridge::{register_dom_bridge, register_layout_api, DomBridgeState, LayoutHost, LayoutSnapshot};
        use rix_dom::Document;
        use std::cell::RefCell;
        use std::rc::Rc;

        struct TestLayoutHost {
            layout_call_count: RefCell<usize>,
            computed_node_call_count: RefCell<usize>,
        }

        impl LayoutHost for TestLayoutHost {
            fn layout(&self, _document: &Document) -> Option<Rc<LayoutSnapshot>> {
                *self.layout_call_count.borrow_mut() += 1;
                Some(Rc::new(LayoutSnapshot::empty()))
            }
            fn computed_style_for_node(
                &self,
                _document: &Document,
                _node: rix_dom::NodeId,
            ) -> Option<rix_css::ComputedStyle> {
                *self.computed_node_call_count.borrow_mut() += 1;
                let mut style = rix_css::ComputedStyle::default();
                style.color = rix_css::color::CssColor::from_rgba_u8(255, 0, 0, 255);
                style.display = rix_css::Display::Block;
                Some(style)
            }
            fn viewport_size(&self) -> (f32, f32) {
                (1280.0, 800.0)
            }
            fn scroll_position(&self) -> (f32, f32) {
                (0.0, 0.0)
            }
            fn request_scroll_position(&self, _x: f32, _y: f32) {}
        }

        let mut runtime = JsRuntime::new(JsRuntimeConfig::default());
        let doc = Rc::new(RefCell::new(Document::new()));
        let dom_state = Rc::new(RefCell::new(DomBridgeState::with_shared_document(doc)));
        let test_host = Rc::new(TestLayoutHost {
            layout_call_count: RefCell::new(0),
            computed_node_call_count: RefCell::new(0),
        });
        register_dom_bridge(runtime.context_mut(), dom_state).expect("dom bridge registered");
        register_layout_api(runtime.context_mut(), test_host.clone()).expect("layout api registered");

        let code = r#"
            const div = document.createElement('div');
            document.appendChild(div);
            const cs = window.getComputedStyle(div);
            const color = cs.color;
            const display = cs.display;
            ({ color, display })
        "#;
        let res = runtime.evaluate(code).expect("script evaluation");
        let obj = res.as_object().expect("returned object");
        let color = obj.get(boa_engine::js_string!("color"), runtime.context_mut()).unwrap();
        assert_eq!(color.to_string(runtime.context_mut()).unwrap().to_std_string_escaped(), "rgb(255, 0, 0)");
        let display = obj.get(boa_engine::js_string!("display"), runtime.context_mut()).unwrap();
        assert_eq!(display.to_string(runtime.context_mut()).unwrap().to_std_string_escaped(), "block");

        assert_eq!(*test_host.layout_call_count.borrow(), 0, "layout() must not be called for non-layout properties!");
        assert!(*test_host.computed_node_call_count.borrow() > 0, "computed_style_for_node must be called!");
    }

    #[test]
    fn test_element_insert_adjacent_and_parent_child_nodes() {
        use crate::dom_bridge::{register_dom_bridge, DomBridgeState};
        use rix_dom::Document;
        use std::cell::RefCell;
        use std::rc::Rc;

        let mut runtime = JsRuntime::new(JsRuntimeConfig::default());
        let doc = Rc::new(RefCell::new(Document::new()));
        let dom_state = Rc::new(RefCell::new(DomBridgeState::with_shared_document(doc)));
        register_dom_bridge(runtime.context_mut(), dom_state).expect("dom bridge registered");

        let code = r#"
            const html = document.createElement('html');
            const body = document.createElement('body');
            document.appendChild(html);
            html.appendChild(body);
            const div = document.createElement('div');
            body.appendChild(div);

            div.insertAdjacentHTML('afterend', '<p id="p1">One</p><p id="p2">Two</p>');
            div.insertAdjacentHTML('beforebegin', '<span id="s1">Before</span>');
            div.insertAdjacentHTML('afterbegin', '<b>InsideStart</b>');
            div.insertAdjacentHTML('beforeend', '<i>InsideEnd</i>');

            const p1 = document.getElementById('p1');
            const p2 = document.getElementById('p2');
            const s1 = document.getElementById('s1');

            p2.before('TextBeforeP2');
            p1.after('TextAfterP1');

            const rep = document.createElement('h1');
            rep.id = 'heading';
            s1.replaceWith(rep);

            body.prepend('PrependToBody');

            const hasHeading = Boolean(document.getElementById('heading'));
            const s1Removed = document.getElementById('s1') === null;

            ({
                hasHeading,
                s1Removed,
                divInner: div.innerHTML,
                bodyCount: body.children.length,
            })
        "#;
        let res = runtime.evaluate(code).expect("script evaluation");
        let obj = res.as_object().expect("returned object");
        let has_heading = obj.get(boa_engine::js_string!("hasHeading"), runtime.context_mut()).unwrap();
        assert!(has_heading.to_boolean());
        let s1_removed = obj.get(boa_engine::js_string!("s1Removed"), runtime.context_mut()).unwrap();
        assert!(s1_removed.to_boolean());
        let div_inner = obj.get(boa_engine::js_string!("divInner"), runtime.context_mut()).unwrap();
        assert_eq!(
            div_inner.to_string(runtime.context_mut()).unwrap().to_std_string_escaped(),
            "<b>InsideStart</b><i>InsideEnd</i>"
        );
    }

    #[test]
    fn test_live_htmlcollection_revision_memoization() {
        use crate::dom_bridge::{register_dom_bridge, DomBridgeState};
        use rix_dom::Document;
        use std::cell::RefCell;
        use std::rc::Rc;

        let mut runtime = JsRuntime::new(JsRuntimeConfig::default());
        let doc = Rc::new(RefCell::new(Document::new()));
        let dom_state = Rc::new(RefCell::new(DomBridgeState::with_shared_document(doc)));
        register_dom_bridge(runtime.context_mut(), dom_state).expect("dom bridge registered");

        let code = r#"
            const container = document.createElement('div');
            document.appendChild(container);

            for (let i = 0; i < 5; i++) {
                const child = document.createElement('span');
                child.className = 'item';
                container.appendChild(child);
            }

            const coll = container.getElementsByTagName('span');
            const initialLen = coll.length;
            const first = coll[0];
            const second = coll[1];

            const newChild = document.createElement('span');
            container.appendChild(newChild);
            const afterLen = coll.length;

            ({
                initialLen,
                afterLen,
                firstTagName: first.tagName,
                secondTagName: second.tagName,
            })
        "#;
        let res = runtime.evaluate(code).expect("script evaluation");
        let obj = res.as_object().expect("returned object");
        let initial_len = obj.get(boa_engine::js_string!("initialLen"), runtime.context_mut()).unwrap();
        assert_eq!(initial_len.to_number(runtime.context_mut()).unwrap(), 5.0);
        let after_len = obj.get(boa_engine::js_string!("afterLen"), runtime.context_mut()).unwrap();
        assert_eq!(after_len.to_number(runtime.context_mut()).unwrap(), 6.0);
    }

    #[test]
    fn test_classlist_idempotent_mutation_bypass() {
        use crate::dom_bridge::{register_dom_bridge, DomBridgeState};
        use rix_dom::Document;
        use std::cell::RefCell;
        use std::rc::Rc;

        let mut runtime = JsRuntime::new(JsRuntimeConfig::default());
        let doc = Rc::new(RefCell::new(Document::new()));
        let dom_state = Rc::new(RefCell::new(DomBridgeState::with_shared_document(doc.clone())));
        register_dom_bridge(runtime.context_mut(), dom_state).expect("dom bridge registered");

        let code = r#"
            const el = document.createElement('div');
            document.appendChild(el);
            el.classList.add('active');
            const rev1 = document.__getRevision();

            el.classList.add('active');
            const rev2 = document.__getRevision();

            el.classList.remove('nonexistent');
            const rev3 = document.__getRevision();

            el.classList.remove('active');
            const rev4 = document.__getRevision();

            ({
                rev1,
                rev2,
                rev3,
                rev4,
            })
        "#;
        let res = runtime.evaluate(code).expect("script evaluation");
        let obj = res.as_object().expect("returned object");
        let rev1 = obj.get(boa_engine::js_string!("rev1"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let rev2 = obj.get(boa_engine::js_string!("rev2"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let rev3 = obj.get(boa_engine::js_string!("rev3"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let rev4 = obj.get(boa_engine::js_string!("rev4"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();

        assert_eq!(rev1, rev2, "Adding existing class must not bump revision");
        assert_eq!(rev2, rev3, "Removing non-existent class must not bump revision");
        assert!(rev4 > rev3, "Removing existing class must bump revision");
    }

    #[test]
    fn test_element_traversal_and_idempotent_set_attribute() {
        use crate::dom_bridge::{register_dom_bridge, DomBridgeState};
        use rix_dom::Document;
        use std::cell::RefCell;
        use std::rc::Rc;

        let mut runtime = JsRuntime::new(JsRuntimeConfig::default());
        let doc = Rc::new(RefCell::new(Document::new()));
        let dom_state = Rc::new(RefCell::new(DomBridgeState::with_shared_document(doc.clone())));
        register_dom_bridge(runtime.context_mut(), dom_state).expect("dom bridge registered");

        let code = r#"
            const container = document.createElement('div');
            document.appendChild(container);

            const span = document.createElement('span');
            const text = document.createTextNode('hello');
            const p = document.createElement('p');

            container.appendChild(span);
            container.appendChild(text);
            container.appendChild(p);

            const firstEl = container.firstElementChild;
            const lastEl = container.lastElementChild;
            const count = container.childElementCount;

            const nextEl = span.nextElementSibling;
            const prevEl = p.previousElementSibling;

            const nextSib = span.nextSibling;
            const prevSib = p.previousSibling;

            span.setAttribute('data-id', 'test');
            const rev1 = document.__getRevision();

            span.setAttribute('data-id', 'test');
            const rev2 = document.__getRevision();

            span.setAttribute('data-id', 'new-test');
            const rev3 = document.__getRevision();

            ({
                firstElTag: firstEl.tagName,
                lastElTag: lastEl.tagName,
                count,
                nextElTag: nextEl.tagName,
                prevElTag: prevEl.tagName,
                nextSibNodeType: nextSib.nodeType,
                prevSibNodeType: prevSib.nodeType,
                rev1,
                rev2,
                rev3,
            })
        "#;
        let res = runtime.evaluate(code).expect("script evaluation");
        let obj = res.as_object().expect("returned object");

        let first_el_tag = obj.get(boa_engine::js_string!("firstElTag"), runtime.context_mut()).unwrap().to_string(runtime.context_mut()).unwrap().to_std_string_escaped();
        let last_el_tag = obj.get(boa_engine::js_string!("lastElTag"), runtime.context_mut()).unwrap().to_string(runtime.context_mut()).unwrap().to_std_string_escaped();
        let count = obj.get(boa_engine::js_string!("count"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let next_el_tag = obj.get(boa_engine::js_string!("nextElTag"), runtime.context_mut()).unwrap().to_string(runtime.context_mut()).unwrap().to_std_string_escaped();
        let prev_el_tag = obj.get(boa_engine::js_string!("prevElTag"), runtime.context_mut()).unwrap().to_string(runtime.context_mut()).unwrap().to_std_string_escaped();
        let next_sib_node_type = obj.get(boa_engine::js_string!("nextSibNodeType"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let prev_sib_node_type = obj.get(boa_engine::js_string!("prevSibNodeType"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let rev1 = obj.get(boa_engine::js_string!("rev1"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let rev2 = obj.get(boa_engine::js_string!("rev2"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let rev3 = obj.get(boa_engine::js_string!("rev3"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();

        assert_eq!(first_el_tag, "SPAN");
        assert_eq!(last_el_tag, "P");
        assert_eq!(count, 2.0);
        assert_eq!(next_el_tag, "P");
        assert_eq!(prev_el_tag, "SPAN");
        assert_eq!(next_sib_node_type, 3.0); 
        assert_eq!(prev_sib_node_type, 3.0); 
        assert_eq!(rev1, rev2, "Idempotent setAttribute must not bump revision");
        assert!(rev3 > rev2, "Changed setAttribute must bump revision");
    }

    #[test]
    fn test_element_scroll_top_and_scroll_left() {
        use crate::dom_bridge::{register_dom_bridge, register_layout_api, DomBridgeState, LayoutHost, LayoutSnapshot};
        use rix_dom::Document;
        use std::cell::RefCell;
        use std::rc::Rc;

        struct DummyLayoutHost;
        impl LayoutHost for DummyLayoutHost {
            fn layout(&self, _document: &Document) -> Option<Rc<LayoutSnapshot>> {
                Some(Rc::new(LayoutSnapshot::empty()))
            }
            fn computed_style_for_node(
                &self,
                _document: &Document,
                _node: rix_dom::NodeId,
            ) -> Option<rix_css::ComputedStyle> {
                Some(rix_css::ComputedStyle::default())
            }
            fn viewport_size(&self) -> (f32, f32) {
                (1280.0, 800.0)
            }
            fn scroll_position(&self) -> (f32, f32) {
                (0.0, 0.0)
            }
            fn request_scroll_position(&self, _x: f32, _y: f32) {}
        }

        let mut runtime = JsRuntime::new(JsRuntimeConfig::default());
        let doc = Rc::new(RefCell::new(Document::new()));
        let dom_state = Rc::new(RefCell::new(DomBridgeState::with_shared_document(doc.clone())));
        register_dom_bridge(runtime.context_mut(), dom_state).expect("dom bridge registered");
        register_layout_api(runtime.context_mut(), Rc::new(DummyLayoutHost)).expect("layout api registered");

        let code = r#"
            const div = document.createElement('div');
            document.appendChild(div);

            const initialTop = div.scrollTop;
            const initialLeft = div.scrollLeft;

            div.scrollTop = 120;
            div.scrollLeft = 45;
            const updatedTop = div.scrollTop;
            const updatedLeft = div.scrollLeft;

            div.scrollBy(10, 20);
            const byTop = div.scrollTop;
            const byLeft = div.scrollLeft;

            div.scrollTo(0, 0);
            const resetTop = div.scrollTop;
            const resetLeft = div.scrollLeft;

            ({
                initialTop,
                initialLeft,
                updatedTop,
                updatedLeft,
                byTop,
                byLeft,
                resetTop,
                resetLeft,
            })
        "#;
        let res = runtime.evaluate(code).expect("script evaluation");
        let obj = res.as_object().expect("returned object");

        let initial_top = obj.get(boa_engine::js_string!("initialTop"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let initial_left = obj.get(boa_engine::js_string!("initialLeft"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let updated_top = obj.get(boa_engine::js_string!("updatedTop"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let updated_left = obj.get(boa_engine::js_string!("updatedLeft"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let by_top = obj.get(boa_engine::js_string!("byTop"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let by_left = obj.get(boa_engine::js_string!("byLeft"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let reset_top = obj.get(boa_engine::js_string!("resetTop"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();
        let reset_left = obj.get(boa_engine::js_string!("resetLeft"), runtime.context_mut()).unwrap().to_number(runtime.context_mut()).unwrap();

        assert_eq!(initial_top, 0.0);
        assert_eq!(initial_left, 0.0);
        assert_eq!(updated_top, 120.0);
        assert_eq!(updated_left, 45.0);
        assert_eq!(by_top, 140.0);
        assert_eq!(by_left, 55.0);
        assert_eq!(reset_top, 0.0);
        assert_eq!(reset_left, 0.0);
    }
}
