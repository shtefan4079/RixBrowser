use boa_engine::{
    Context, JsArgs, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue,
    builtins::{promise::ResolvingFunctions, typed_array::TypedArrayKind},
    job::PromiseJob,
    js_string,
    native_function::NativeFunction,
    object::{
        FunctionObjectBuilder, ObjectInitializer,
        builtins::{JsPromise, JsTypedArray, JsUint8Array},
    },
    property::{Attribute, PropertyDescriptor},
};
use rix_core::url::RixUrl;
use rix_storage::StorageEngine;
use rix_storage::models::Cookie;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod abort;
mod animation_frames;
mod blob;
mod css;
mod event_loop;
mod fetch;
mod history;
mod idle_callbacks;
mod location;
mod platform;
mod resource_trace;
mod response;
mod storage;
mod streams;
mod text_codec;
mod timers;
mod url_api;
mod user_timing;

#[derive(Clone)]
pub(crate) struct EventDispatchHooks {
    pub(crate) dispatch_window: JsObject,
    pub(crate) dispatch_promise_rejection: JsObject,
    pub(crate) set_path: JsObject,
    pub(crate) clear_path: JsObject,
}

use abort::*;
use animation_frames::*;
use css::*;
pub use event_loop::*;
use fetch::*;
pub use fetch::{get_shared_network_client, set_shared_network_client};
use history::*;
pub use history::{HistoryChange, HistoryChangeKind, traverse_session_history};
use idle_callbacks::*;
use location::*;
pub use location::{PageNavigationKind, PageNavigationRequest};
pub use platform::*;
pub use resource_trace::{
    ResourceLoadKind, ResourceLoadRecord, redact_diagnostic, redact_resource_url,
};
pub use storage::StorageMutation;
use storage::*;
use timers::*;

const MAX_RESOURCE_LOAD_RECORDS: usize = 512;

#[derive(Clone, Copy)]
struct PerformanceClock {
    started: Instant,
    elapsed_at_start_ms: f64,
    time_origin: f64,
    navigation: Option<rix_net::NavigationTiming>,
    lifecycle: [Option<f64>; 7],
}

impl PerformanceClock {
    fn elapsed_ms(&self) -> f64 {
        self.elapsed_at_start_ms + self.started.elapsed().as_secs_f64() * 1000.0
    }
}

fn performance_now(_this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let clock = context
        .get_data::<PerformanceClock>()
        .ok_or_else(|| JsNativeError::typ().with_message("Performance clock is unavailable"))?;
    Ok(JsValue::new(clock.elapsed_ms()))
}

fn performance_time_origin(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let clock = context
        .get_data::<PerformanceClock>()
        .ok_or_else(|| JsNativeError::typ().with_message("Performance clock is unavailable"))?;
    Ok(JsValue::new(clock.time_origin))
}

#[derive(Clone, Copy, Debug)]
pub enum NavigationMilestone {
    DomLoading,
    DomInteractive,
    DomContentLoadedEventStart,
    DomContentLoadedEventEnd,
    DomComplete,
    LoadEventStart,
    LoadEventEnd,
}

const LIFECYCLE_FIELDS: [&str; 7] = [
    "domLoading",
    "domInteractive",
    "domContentLoadedEventStart",
    "domContentLoadedEventEnd",
    "domComplete",
    "loadEventStart",
    "loadEventEnd",
];

pub fn set_navigation_performance(
    context: &mut Context,
    timing: rix_net::NavigationTiming,
    elapsed_since_origin_ms: f64,
) -> Result<(), JsError> {
    let offsets = [
        timing.fetch_start_ms,
        timing.request_start_ms,
        timing.response_start_ms,
        timing.response_end_ms,
    ];
    if !timing.time_origin_ms.is_finite()
        || timing.time_origin_ms < 0.0
        || !elapsed_since_origin_ms.is_finite()
        || elapsed_since_origin_ms < timing.response_end_ms
        || offsets
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        || offsets.windows(2).any(|pair| pair[0] > pair[1])
        || [timing.redirect_start_ms, timing.redirect_end_ms]
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite() || value < 0.0)
    {
        return Err(JsNativeError::range()
            .with_message("Navigation timing must contain finite, ordered measurements")
            .into());
    }
    context.insert_data(PerformanceClock {
        started: Instant::now(),
        elapsed_at_start_ms: elapsed_since_origin_ms,
        time_origin: timing.time_origin_ms,
        navigation: Some(timing),
        lifecycle: [None; 7],
    });
    let performance = context
        .global_object()
        .get(js_string!("performance"), context)?;
    if let Some(performance) = performance.as_object() {
        if performance
            .get(js_string!("timing"), context)?
            .is_undefined()
        {
            let timing = navigation_timing_object(context);
            performance.define_property_or_throw(
                js_string!("timing"),
                PropertyDescriptor::builder()
                    .value(timing)
                    .writable(false)
                    .enumerable(true)
                    .configurable(true)
                    .build(),
                context,
            )?;
        }
    }
    Ok(())
}

pub fn mark_navigation_milestone(context: &mut Context, milestone: NavigationMilestone) {
    let Some(mut clock) = context.get_data::<PerformanceClock>().copied() else {
        return;
    };
    if clock.navigation.is_some() {
        clock.lifecycle[milestone as usize] = Some(clock.elapsed_ms());
        context.insert_data(clock);
    }
}

fn navigation_timestamp(context: &Context, field: &str) -> f64 {
    let Some(clock) = context.get_data::<PerformanceClock>() else {
        return 0.0;
    };
    let Some(timing) = clock.navigation else {
        return 0.0;
    };
    let offset = match field {
        "navigationStart" => Some(0.0),
        "fetchStart" => Some(timing.fetch_start_ms),
        "requestStart" => Some(timing.request_start_ms),
        "responseStart" => Some(timing.response_start_ms),
        "responseEnd" => Some(timing.response_end_ms),
        "redirectStart" => timing.redirect_start_ms,
        "redirectEnd" => timing.redirect_end_ms,
        _ => LIFECYCLE_FIELDS
            .iter()
            .position(|name| *name == field)
            .and_then(|index| clock.lifecycle[index]),
    };
    offset.map_or(0.0, |offset| (clock.time_origin + offset).floor())
}

fn navigation_timing_object(context: &mut Context) -> JsObject {
    let timing = ObjectInitializer::new(context).build();
    for field in [
        "navigationStart",
        "fetchStart",
        "requestStart",
        "responseStart",
        "responseEnd",
        "redirectStart",
        "redirectEnd",
    ]
    .into_iter()
    .chain(LIFECYCLE_FIELDS)
    {
        let getter = FunctionObjectBuilder::new(
            context.realm(),
            NativeFunction::from_copy_closure(move |_this, _args, context| {
                Ok(JsValue::new(navigation_timestamp(context, field)))
            }),
        )
        .name(js_string!(field))
        .build();
        timing
            .define_property_or_throw(
                js_string!(field),
                PropertyDescriptor::builder()
                    .get(getter)
                    .enumerable(true)
                    .build(),
                context,
            )
            .expect("fresh navigation timing property");
    }
    timing
}

#[derive(Clone)]
pub enum TimerCallback {
    Function(JsObject),
    Code(String),
}

#[derive(Clone)]
pub struct Timer {
    pub id: u32,
    pub callback: TimerCallback,
    pub args: Vec<JsValue>,
    pub fire_at: Instant,
    pub interval: Option<Duration>,
}

#[derive(Clone)]
pub struct IdleCallback {
    pub id: u32,
    pub callback: JsObject,
    pub timeout_at: Option<Instant>,
}

struct FetchCancellation {
    cancel_sender: tokio::sync::watch::Sender<bool>,
    reason: Option<JsValue>,
}

#[derive(Default)]
pub struct WebApiState {
    pub next_timer_id: u32,
    pub timers: HashMap<u32, Timer>,
    next_idle_callback_id: u32,
    pending_idle_callbacks: VecDeque<IdleCallback>,
    runnable_idle_callbacks: VecDeque<IdleCallback>,
    idle_period_deadline: Option<Instant>,
    idle_period_active: bool,
    next_animation_frame_id: u32,
    animation_frame_callback_order: VecDeque<u32>,
    animation_frame_callbacks: HashMap<u32, JsObject>,
    animation_frame_requested: bool,
    animation_frame_request_notification: bool,
    pub microtasks: VecDeque<(JsObject, Vec<JsValue>)>,
    pending_fetches: Vec<PendingFetch>,
    next_fetch_abort_id: u64,
    fetch_cancellations: HashMap<u64, FetchCancellation>,
    completed_resource_loads: VecDeque<ResourceLoadRecord>,
    resource_loads_truncated: bool,
    jobs: Option<Rc<crate::jobs::BrowserJobExecutor>>,
    promise_rejections: Option<Rc<RefCell<crate::jobs::PromiseRejectionTracker>>>,
    pub local_storage: BTreeMap<String, String>,
    pub session_storage: BTreeMap<String, String>,
    pub current_url: Option<RixUrl>,
    pub navigation_target: Option<String>,
    pub navigation_replaces_entry: bool,
    pub reload_requested: bool,
    pub history_changes: VecDeque<HistoryChange>,
    pub cookie_store: Option<StorageEngine>,
    pub updated_cookies: VecDeque<Cookie>,
    pub storage_mutations: VecDeque<StorageMutation>,
    pub fetch_head_timeout: Duration,
}

impl WebApiState {
    pub fn new(initial_url: Option<RixUrl>) -> Self {
        let cookie_store = StorageEngine::in_memory().ok();
        let mut local_storage = BTreeMap::new();
        if let (Some(store), Some(url)) = (&cookie_store, &initial_url) {
            let origin = url.origin();
            if let Ok(entries) = store.get_all_local_storage(&origin) {
                local_storage.extend(entries);
            }
        }
        Self {
            next_timer_id: 1,
            timers: HashMap::new(),
            next_idle_callback_id: 0,
            pending_idle_callbacks: VecDeque::new(),
            runnable_idle_callbacks: VecDeque::new(),
            idle_period_deadline: None,
            idle_period_active: false,
            next_animation_frame_id: 0,
            animation_frame_callback_order: VecDeque::new(),
            animation_frame_callbacks: HashMap::new(),
            animation_frame_requested: false,
            animation_frame_request_notification: false,
            microtasks: VecDeque::new(),
            pending_fetches: Vec::new(),
            next_fetch_abort_id: 1,
            fetch_cancellations: HashMap::new(),
            completed_resource_loads: VecDeque::new(),
            resource_loads_truncated: false,
            jobs: None,
            promise_rejections: None,
            local_storage,
            session_storage: BTreeMap::new(),
            current_url: initial_url,
            navigation_target: None,
            navigation_replaces_entry: false,
            reload_requested: false,
            history_changes: VecDeque::new(),
            cookie_store,
            updated_cookies: VecDeque::new(),
            storage_mutations: VecDeque::new(),
            fetch_head_timeout: Duration::from_secs(12),
        }
    }

    pub fn with_cookie_store(mut self, cookie_store: StorageEngine) -> Self {
        if let Some(url) = &self.current_url {
            let origin = url.origin();
            if let Ok(entries) = cookie_store.get_all_local_storage(&origin) {
                for (k, v) in entries {
                    self.local_storage.entry(k).or_insert(v);
                }
            }
        }
        self.cookie_store = Some(cookie_store);
        self
    }

    pub fn set_fetch_head_timeout(&mut self, timeout: Duration) {
        self.fetch_head_timeout = timeout;
    }

    pub fn set_cookie_store(&mut self, cookie_store: StorageEngine) {
        if let Some(url) = &self.current_url {
            let origin = url.origin();
            if let Ok(entries) = cookie_store.get_all_local_storage(&origin) {
                for (k, v) in entries {
                    self.local_storage.entry(k).or_insert(v);
                }
            }
        }
        self.cookie_store = Some(cookie_store);
    }

    pub fn record_cookie_update(&mut self, cookie: Cookie) {
        if self.updated_cookies.len() < 512 {
            self.updated_cookies.push_back(cookie);
        }
    }

    pub fn take_updated_cookies(&mut self) -> Vec<Cookie> {
        self.updated_cookies.drain(..).collect()
    }

    pub fn record_storage_mutation(&mut self, mutation: StorageMutation) {
        if self.storage_mutations.len() < 512 {
            self.storage_mutations.push_back(mutation);
        }
    }

    pub fn take_storage_mutations(&mut self) -> Vec<StorageMutation> {
        self.storage_mutations.drain(..).collect()
    }

    pub fn record_navigation(&mut self, input: &str, kind: PageNavigationKind) {
        if kind == PageNavigationKind::Reload {
            self.reload_requested = true;
            return;
        }
        let resolved = resolve_navigation_url(self, input);
        self.navigation_target = Some(resolved);
        self.navigation_replaces_entry = kind == PageNavigationKind::Replace;
    }

    pub fn record_history_change(
        &mut self,
        url: &str,
        kind: HistoryChangeKind,
        index: u32,
        length: u32,
    ) {
        if self.history_changes.len() < 64 {
            self.history_changes.push_back(HistoryChange {
                url: url.to_string(),
                kind,
                index,
                length,
            });
        }
    }

    pub fn take_history_changes(&mut self) -> Vec<HistoryChange> {
        self.history_changes.drain(..).collect()
    }

    pub fn take_navigation_request(&mut self) -> Option<PageNavigationRequest> {
        if self.reload_requested {
            self.reload_requested = false;
            self.navigation_target = None;
            self.navigation_replaces_entry = false;
            return Some(PageNavigationRequest {
                url: String::new(),
                kind: PageNavigationKind::Reload,
            });
        }
        let url = self.navigation_target.take()?;
        let kind = if std::mem::take(&mut self.navigation_replaces_entry) {
            PageNavigationKind::Replace
        } else {
            PageNavigationKind::Assign
        };
        Some(PageNavigationRequest { url, kind })
    }

    pub fn schedule_timer(
        &mut self,
        callback: TimerCallback,
        args: Vec<JsValue>,
        delay: Duration,
        interval: Option<Duration>,
    ) -> u32 {
        let fire_at = Instant::now() + delay;
        if let Some(deadline) = self.idle_period_deadline {
            if fire_at < deadline {
                self.idle_period_deadline = Some(fire_at);
            }
        }
        let id = self.next_timer_id;
        self.next_timer_id = self.next_timer_id.wrapping_add(1).max(1);
        self.timers.insert(
            id,
            Timer {
                id,
                callback,
                args,
                fire_at,
                interval,
            },
        );
        id
    }

    pub fn schedule_function_timer(
        &mut self,
        callback: JsObject,
        delay: Duration,
        interval: Option<Duration>,
    ) -> u32 {
        self.schedule_timer(
            TimerCallback::Function(callback),
            Vec::new(),
            delay,
            interval,
        )
    }

    pub fn cancel_timer(&mut self, id: u32) {
        self.timers.remove(&id);
    }

    fn schedule_idle_callback(&mut self, callback: JsObject, timeout: Option<Duration>) -> u32 {
        self.next_idle_callback_id = self.next_idle_callback_id.wrapping_add(1);
        let id = self.next_idle_callback_id;
        self.pending_idle_callbacks.push_back(IdleCallback {
            id,
            callback,
            timeout_at: timeout.map(|delay| Instant::now() + delay),
        });
        id
    }

    fn cancel_idle_callback(&mut self, id: u32) {
        self.pending_idle_callbacks
            .retain(|callback| callback.id != id);
        self.runnable_idle_callbacks
            .retain(|callback| callback.id != id);
    }

    fn schedule_animation_frame(&mut self, callback: JsObject) -> u32 {
        let id = loop {
            self.next_animation_frame_id = self.next_animation_frame_id.wrapping_add(1);
            if !self
                .animation_frame_callbacks
                .contains_key(&self.next_animation_frame_id)
            {
                break self.next_animation_frame_id;
            }
        };
        if !self.animation_frame_requested {
            self.animation_frame_requested = true;
            self.animation_frame_request_notification = true;
        }
        self.animation_frame_callback_order.push_back(id);
        self.animation_frame_callbacks.insert(id, callback);
        id
    }

    fn cancel_animation_frame(&mut self, id: u32) {
        self.animation_frame_callbacks.remove(&id);
    }

    fn take_animation_frame_request_notification(&mut self) -> bool {
        std::mem::take(&mut self.animation_frame_request_notification)
    }

    fn begin_animation_frame(&mut self) -> Vec<u32> {
        self.animation_frame_requested = false;
        self.animation_frame_request_notification = false;
        self.animation_frame_callback_order.drain(..).collect()
    }

    fn take_animation_frame_callback(&mut self, id: u32) -> Option<JsObject> {
        self.animation_frame_callbacks.remove(&id)
    }

    fn has_idle_callbacks(&self) -> bool {
        !self.pending_idle_callbacks.is_empty() || !self.runnable_idle_callbacks.is_empty()
    }

    fn earliest_idle_timeout(&self) -> Option<Instant> {
        self.pending_idle_callbacks
            .iter()
            .chain(&self.runnable_idle_callbacks)
            .filter_map(|callback| callback.timeout_at)
            .min()
    }

    fn end_idle_period_early(&mut self) {
        self.idle_period_active = false;
        self.idle_period_deadline = None;
    }

    fn take_timed_out_idle_callback(&mut self, now: Instant) -> Option<IdleCallback> {
        let selected = self
            .pending_idle_callbacks
            .iter()
            .chain(&self.runnable_idle_callbacks)
            .filter(|callback| callback.timeout_at.is_some_and(|timeout| timeout <= now))
            .min_by_key(|callback| (callback.timeout_at, callback.id))
            .cloned()?;
        self.cancel_idle_callback(selected.id);
        self.end_idle_period_early();
        Some(selected)
    }

    fn start_idle_period_if_needed(&mut self, now: Instant, next_timer_deadline: Option<Instant>) {
        if self
            .idle_period_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            self.idle_period_deadline = None;
            self.idle_period_active = false;
        }
        if self.idle_period_deadline.is_some() {
            return;
        }
        if !self.has_idle_callbacks() {
            return;
        }

        let mut deadline = now + Duration::from_millis(50);
        if let Some(timer_deadline) = next_timer_deadline.filter(|at| *at > now) {
            deadline = deadline.min(timer_deadline);
        }
        self.idle_period_deadline = Some(deadline);
        self.idle_period_active = true;
        self.runnable_idle_callbacks
            .append(&mut self.pending_idle_callbacks);
    }

    fn take_idle_callback(
        &mut self,
        now: Instant,
        next_timer_deadline: Option<Instant>,
    ) -> Option<(IdleCallback, Instant)> {
        self.start_idle_period_if_needed(now, next_timer_deadline);
        let deadline = self.idle_period_deadline?;
        if !self.idle_period_active || deadline <= now {
            return None;
        }
        self.runnable_idle_callbacks
            .pop_front()
            .map(|callback| (callback, deadline))
    }

    fn next_idle_deadline(&self, now: Instant) -> Option<Instant> {
        let timeout = self.earliest_idle_timeout();
        if !self.has_idle_callbacks() {
            return timeout;
        }

        let idle_work = if !self.runnable_idle_callbacks.is_empty() || !self.pending_idle_callbacks.is_empty() {
            Some(now)
        } else {
            None
        };
        idle_work.into_iter().chain(timeout).min()
    }

    fn register_fetch_cancellation(
        &mut self,
        cancel_sender: tokio::sync::watch::Sender<bool>,
    ) -> u64 {
        let id = loop {
            let id = self.next_fetch_abort_id;
            self.next_fetch_abort_id = self.next_fetch_abort_id.wrapping_add(1).max(1);
            if !self.fetch_cancellations.contains_key(&id) {
                break id;
            }
        };
        self.fetch_cancellations.insert(
            id,
            FetchCancellation {
                cancel_sender,
                reason: None,
            },
        );
        id
    }

    fn abort_fetch(&mut self, id: u64, reason: JsValue) {
        if let Some(cancellation) = self.fetch_cancellations.get_mut(&id) {
            if cancellation.reason.is_none() {
                cancellation.reason = Some(reason);
                cancellation.cancel_sender.send_replace(true);
            }
        }
    }

    fn fetch_abort_reason(&self, id: u64) -> Option<JsValue> {
        self.fetch_cancellations
            .get(&id)
            .and_then(|cancellation| cancellation.reason.clone())
    }

    fn finish_fetch_cancellation(&mut self, id: u64) {
        self.fetch_cancellations.remove(&id);
    }

    pub fn queue_microtask(&mut self, callback: JsObject, args: Vec<JsValue>) {
        self.microtasks.push_back((callback, args));
    }

    pub fn record_resource_load(&mut self, record: ResourceLoadRecord) {
        if self.completed_resource_loads.len() == MAX_RESOURCE_LOAD_RECORDS {
            self.completed_resource_loads.pop_front();
            self.resource_loads_truncated = true;
        }
        self.completed_resource_loads.push_back(record);
    }

    pub fn take_resource_loads(&mut self) -> Vec<ResourceLoadRecord> {
        self.completed_resource_loads.drain(..).collect()
    }

    pub fn take_resource_loads_truncated(&mut self) -> bool {
        std::mem::take(&mut self.resource_loads_truncated)
    }

    pub fn pending_fetch_count(&self) -> usize {
        self.pending_fetches.len()
    }

    pub fn has_pending_tasks(&self) -> bool {
        !self.timers.is_empty()
            || self.has_idle_callbacks()
            || !self.animation_frame_callbacks.is_empty()
            || !self.microtasks.is_empty()
            || !self.pending_fetches.is_empty()
            || self.jobs.as_ref().is_some_and(|jobs| jobs.has_pending())
            || self
                .promise_rejections
                .as_ref()
                .is_some_and(|tracker| tracker.borrow().has_pending_events())
    }
}

pub fn register_web_apis(
    context: &mut Context,
    state: Rc<RefCell<WebApiState>>,
) -> Result<(), JsError> {
    {
        let mut state = state.borrow_mut();
        state.jobs = context
            .get_data::<Rc<crate::jobs::BrowserJobExecutor>>()
            .cloned();
        state.promise_rejections = context
            .get_data::<Rc<RefCell<crate::jobs::PromiseRejectionTracker>>>()
            .cloned();
    }
    context.insert_data(state);

    register_timer_functions(context)?;

    register_navigator(context)?;

    register_location(context)?;

    register_fetch(context)?;

    register_standard_web_platform_apis(context)?;

    register_local_storage(context)?;

    register_idle_callback_api(context)?;

    register_animation_frame_api(context)?;

    Ok(())
}
