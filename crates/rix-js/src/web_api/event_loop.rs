use super::*;

enum FetchStep {
    Head(StreamHead),
    Chunk(Vec<u8>),
    End,
    Failed(String),
    Aborted(JsValue),
    Flush,
}

enum DrainOutcome {
    Pending,
    Finished,
}

fn is_worker_trace() -> bool {
    static TRACE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    match TRACE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let active = std::env::var_os("RIX_WORKER_TRACE").is_some();
            TRACE.store(if active { 1 } else { 2 }, std::sync::atomic::Ordering::Relaxed);
            active
        }
    }
}

pub struct EventLoop {
    state: Rc<RefCell<WebApiState>>,
    task_errors: Vec<String>,
}

impl EventLoop {
    pub fn new(state: Rc<RefCell<WebApiState>>) -> Self {
        Self {
            state,
            task_errors: Vec::new(),
        }
    }

    pub fn take_task_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.task_errors)
    }

    pub fn take_animation_frame_request(&mut self) -> bool {
        self.state
            .borrow_mut()
            .take_animation_frame_request_notification()
    }

    pub fn has_requested_animation_frame(&self) -> bool {
        self.state.borrow().animation_frame_requested
    }

    pub fn run_animation_frame(&mut self, timestamp_ms: f64, context: &mut Context) -> usize {
        if !timestamp_ms.is_finite() || timestamp_ms < 0.0 {
            tracing::warn!(timestamp_ms, "ignoring invalid animation frame timestamp");
            return 0;
        }
        let callback_ids = self.state.borrow_mut().begin_animation_frame();
        let mut invoked = 0;
        for id in callback_ids {
            let callback = self.state.borrow_mut().take_animation_frame_callback(id);
            let Some(callback) = callback else {
                continue;
            };
            let this: JsValue = context.global_object().into();
            if let Err(error) = callback.call(&this, &[JsValue::new(timestamp_ms)], context) {
                self.report_task_error(error);
            }
            invoked += 1;
        }
        invoked
    }

    pub fn run_animation_frame_at_now(&mut self, context: &mut Context) -> usize {
        let timestamp_ms = context
            .get_data::<PerformanceClock>()
            .map(PerformanceClock::elapsed_ms)
            .unwrap_or(0.0);
        self.run_animation_frame(timestamp_ms, context)
    }

    pub fn run_rendering_step(&mut self, timestamp_ms: f64, context: &mut Context) -> usize {
        self.run_rendering_step_with_progress(timestamp_ms, context, |_| {})
    }

    pub fn run_rendering_step_with_progress(
        &mut self,
        timestamp_ms: f64,
        context: &mut Context,
        mut phase_started: impl FnMut(&'static str),
    ) -> usize {
        if self.has_pending_microtasks() {
            phase_started("pre-rendering microtask checkpoint");
            self.drain_microtasks(context);
        }

        phase_started("animation frame callbacks");
        let invoked = self.run_animation_frame(timestamp_ms, context);

        if self.has_pending_microtasks() {
            phase_started("animation frame microtask checkpoint");
            self.drain_microtasks(context);
        }

        phase_started("rendering layout observer checkpoint");
        if let Err(error) = crate::dom_bridge::run_layout_observers(context) {
            self.report_task_error(error);
        }

        if self.has_pending_microtasks() {
            phase_started("post-observer microtask checkpoint");
            self.drain_microtasks(context);
        }

        self.dispatch_promise_rejection_events(context);
        invoked
    }

    fn report_task_error(&mut self, error: JsError) {
        let message = error.to_string();
        tracing::warn!(%message, "uncaught page task exception");
        self.task_errors.push(message);
    }

    fn run_jobs(&mut self, context: &mut Context) {
        if let Err(error) = context.run_jobs() {
            self.report_task_error(error);
        }
        self.task_errors.extend(crate::exceptions::take(context));
    }

    pub fn has_pending(&self) -> bool {
        self.state.borrow().has_pending_tasks()
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        let state = self.state.borrow();
        let timer = state.timers.values().map(|timer| timer.fire_at).min();
        let idle = state.next_idle_deadline(Instant::now());
        let checkpoint = if !state.microtasks.is_empty()
            || state.jobs.as_ref().is_some_and(|jobs| jobs.has_pending())
            || state
                .promise_rejections
                .as_ref()
                .is_some_and(|tracker| tracker.borrow().has_pending_events())
        {
            Some(Instant::now())
        } else if !state.pending_fetches.is_empty() {
            Some(Instant::now() + Duration::from_millis(8))
        } else {
            None
        };
        timer.into_iter().chain(idle).chain(checkpoint).min()
    }

    pub fn debug_pending_counts(&self) -> (usize, usize, bool, usize, usize) {
        let state = self.state.borrow();
        let (jobs, engine_jobs) = state
            .jobs
            .as_ref()
            .map(|jobs| jobs.pending_counts())
            .unwrap_or((0, false));
        (
            state.microtasks.len(),
            jobs,
            engine_jobs,
            state.timers.len(),
            state.pending_fetches.len(),
        )
    }

    fn drain_microtasks(&mut self, context: &mut Context) -> bool {
        let started = Instant::now();
        let mut total_drained = 0;
        while total_drained < 16_384 {
            let next = self.state.borrow_mut().microtasks.pop_front();
            if let Some((cb, args)) = next {
                total_drained += 1;
                let this: JsValue = context.global_object().into();
                if let Err(error) = cb.call(&this, &args, context) {
                    self.report_task_error(error);
                }
            } else if self.state.borrow().jobs.as_ref().is_some_and(|jobs| jobs.has_pending()) {
                self.run_jobs(context);
                total_drained += 1;
            } else {
                break;
            }
            if started.elapsed() >= Duration::from_millis(500) {
                break;
            }
        }
        let elapsed = started.elapsed();
        if is_worker_trace() && elapsed >= Duration::from_millis(50) {
            eprintln!(
                "[rix-microtask-slow] elapsed={}ms drained={}",
                elapsed.as_millis(),
                total_drained
            );
        }
        !self.has_pending_microtasks()
    }

    pub fn has_pending_microtasks(&self) -> bool {
        let state = self.state.borrow();
        !state.microtasks.is_empty() || state.jobs.as_ref().is_some_and(|jobs| jobs.has_pending())
    }

    fn dispatch_promise_rejection_events(&mut self, context: &mut Context) {
        let (tracker, jobs_pending) = {
            let state = self.state.borrow();
            (
                state.promise_rejections.clone(),
                state.jobs.as_ref().is_some_and(|jobs| jobs.has_pending()),
            )
        };
        if jobs_pending {
            return;
        }
        let Some(tracker) = tracker else {
            return;
        };
        let Some(dispatcher) = context
            .get_data::<EventDispatchHooks>()
            .map(|hooks| hooks.dispatch_promise_rejection.clone())
        else {
            return;
        };

        let unhandled = tracker.borrow_mut().take_pending_unhandled();
        let handled = tracker.borrow_mut().take_pending_handled();
        for (event_type, records) in [
            ("unhandledrejection", unhandled),
            ("rejectionhandled", handled),
        ] {
            for record in records {
                let result = dispatcher.call(
                    &JsValue::undefined(),
                    &[
                        js_string!(event_type).into(),
                        record.promise.into(),
                        record.reason,
                    ],
                    context,
                );
                match result {
                    Ok(value)
                        if event_type == "unhandledrejection"
                            && value.as_boolean() == Some(true) =>
                    {
                        tracing::warn!("unhandled promise rejection was not canceled by the page");
                    }
                    Ok(_) => {}
                    Err(error) => self.report_task_error(error),
                }
            }
        }
    }

    fn poll_fetches(&mut self, context: &mut Context) -> usize {
        let mut steps: Vec<(PendingFetch, FetchStep)> = Vec::new();
        let mut pending = {
            let mut state = self.state.borrow_mut();
            std::mem::take(&mut state.pending_fetches)
        };
        for mut request in pending.drain(..) {
            if let Some(step) = self.fetch_abort_step(&mut request, context) {
                steps.push((request, step));
                continue;
            }

            if request.upload.is_some() {
                if let Some(step) = self.pump_fetch_upload(&mut request, context) {
                    steps.push((request, step));
                    continue;
                }
            }

            if let Some(step) = self.fetch_abort_step(&mut request, context) {
                steps.push((request, step));
                continue;
            }

            let body_has_work = request.body.as_ref().is_some_and(|body| {
                !body.pending.is_empty() || body.end_pending || body.error_pending.is_some()
            });
            match request.result.try_recv() {
                Ok(FetchProgress::Head(head)) => steps.push((request, FetchStep::Head(head))),
                Ok(FetchProgress::Chunk(bytes)) => {
                    steps.push((request, FetchStep::Chunk(bytes)));
                }
                Ok(FetchProgress::End) => steps.push((request, FetchStep::End)),
                Ok(FetchProgress::Failed(message)) => {
                    steps.push((request, FetchStep::Failed(message)));
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    if body_has_work {
                        steps.push((request, FetchStep::Flush));
                    } else {
                        self.state.borrow_mut().pending_fetches.push(request);
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    steps.push((
                        request,
                        FetchStep::Failed("fetch service disconnected".to_string()),
                    ));
                }
            }
        }
        let mut count = 0;
        for (request, step) in steps {
            count += 1;
            if let Some(request) = self.apply_fetch_step(request, step, context) {
                self.state.borrow_mut().pending_fetches.push(request);
            }
        }
        count
    }

    fn fetch_abort_step(
        &mut self,
        request: &mut PendingFetch,
        context: &mut Context,
    ) -> Option<FetchStep> {
        let reason = request
            .abort_id
            .and_then(|id| self.state.borrow().fetch_abort_reason(id))?;
        {
            let mut state = self.state.borrow_mut();
            state.record_resource_load(ResourceLoadRecord::new(
                ResourceLoadKind::Fetch,
                &request.method,
                &request.request_url,
                None,
                None,
                request.started_at.elapsed(),
                None,
                Some("aborted"),
            ));
            if let Some(id) = request.abort_id {
                state.finish_fetch_cancellation(id);
            }
        }
        if let Some(mut upload) = request.upload.take() {
            if let Err(error) = abort_request_body(&mut upload, reason.clone(), context) {
                self.report_task_error(error);
            }
        }
        Some(FetchStep::Aborted(reason))
    }

    fn pump_fetch_upload(
        &mut self,
        request: &mut PendingFetch,
        context: &mut Context,
    ) -> Option<FetchStep> {
        let Some(upload) = request.upload.as_mut() else {
            return None;
        };
        match pump_request_body(upload, context) {
            Ok(UploadProgress::Running) => None,
            Ok(UploadProgress::Ended) => {
                request.upload = None;
                None
            }
            Ok(UploadProgress::Failed(message)) => Some(FetchStep::Failed(message)),
            Err(error) => {
                self.report_task_error(error);
                Some(FetchStep::Failed(
                    "fetch: request body stream failed".to_string(),
                ))
            }
        }
    }

    fn apply_fetch_step(
        &mut self,
        mut request: PendingFetch,
        step: FetchStep,
        context: &mut Context,
    ) -> Option<PendingFetch> {
        match step {
            FetchStep::Head(head) => {
                let (response, body) =
                    match build_fetch_body_stream(&head, request.abort_id, context) {
                        Ok(built) => built,
                        Err(error) => {
                            self.finish_fetch_request(&mut request, context);
                            self.detach_abort_signal(&request, context);
                            let reason = match error.into_opaque(context) {
                                Ok(reason) => reason,
                                Err(error) => {
                                    self.report_task_error(error);
                                    JsValue::undefined()
                                }
                            };
                            if let Err(error) = request.resolvers.reject.call(
                                &JsValue::undefined(),
                                &[reason],
                                context,
                            ) {
                                self.report_task_error(error);
                            }
                            return None;
                        }
                    };
                self.record_response_cookies(&request, &head);
                request.body = Some(body);
                if let Err(error) = request.resolvers.resolve.call(
                    &JsValue::undefined(),
                    &[response.into()],
                    context,
                ) {
                    self.report_task_error(error);
                    self.finish_fetch_request(&mut request, context);
                    self.detach_abort_signal(&request, context);
                    return None;
                }
                Some(request)
            }
            FetchStep::Chunk(bytes) => {
                if let Some(body) = request.body.as_mut() {
                    body.pending.push_back(bytes);
                }
                match self.drain_body(&mut request, context) {
                    DrainOutcome::Pending => Some(request),
                    DrainOutcome::Finished => None,
                }
            }
            FetchStep::Flush => match self.drain_body(&mut request, context) {
                DrainOutcome::Pending => Some(request),
                DrainOutcome::Finished => None,
            },
            FetchStep::End => {
                if let Some(body) = request.body.as_mut() {
                    body.end_pending = true;
                }
                match self.drain_body(&mut request, context) {
                    DrainOutcome::Pending => Some(request),
                    DrainOutcome::Finished => None,
                }
            }
            FetchStep::Failed(message) => {
                if request.body.is_some() {
                    let reason: JsValue = JsNativeError::typ()
                        .with_message(message.clone())
                        .into_opaque(context)
                        .into();
                    if let Some(body) = request.body.as_mut() {
                        body.error_pending = Some(StreamError {
                            reason,
                            trace: Some(message),
                        });
                    }
                    return match self.drain_body(&mut request, context) {
                        DrainOutcome::Pending => Some(request),
                        DrainOutcome::Finished => None,
                    };
                }
                let reason: JsValue = JsNativeError::typ()
                    .with_message(message.clone())
                    .into_opaque(context)
                    .into();
                self.record_fetch_trace(&request, None, None, None, Some(message));
                self.finish_fetch_request(&mut request, context);
                self.detach_abort_signal(&request, context);
                if let Err(error) =
                    request
                        .resolvers
                        .reject
                        .call(&JsValue::undefined(), &[reason], context)
                {
                    self.report_task_error(error);
                }
                None
            }
            FetchStep::Aborted(reason) => {
                let reason = if reason.is_undefined() {
                    match super::fetch::default_abort_reason(context) {
                        Ok(reason) => reason,
                        Err(error) => {
                            self.report_task_error(error);
                            JsValue::undefined()
                        }
                    }
                } else {
                    reason
                };
                if request.body.is_some() {
                    if let Some(body) = request.body.as_mut() {
                        body.error_pending = Some(StreamError {
                            reason,
                            trace: None,
                        });
                    }
                    return match self.drain_body(&mut request, context) {
                        DrainOutcome::Pending => Some(request),
                        DrainOutcome::Finished => None,
                    };
                }
                self.finish_fetch_request(&mut request, context);
                self.detach_abort_signal(&request, context);
                if let Err(error) =
                    request
                        .resolvers
                        .reject
                        .call(&JsValue::undefined(), &[reason], context)
                {
                    self.report_task_error(error);
                }
                None
            }
        }
    }

    fn finish_fetch_request(&mut self, request: &mut PendingFetch, context: &mut Context) {
        if let Some(mut upload) = request.upload.take() {
            if let Err(error) = abort_request_body(&mut upload, JsValue::undefined(), context) {
                self.report_task_error(error);
            }
        }
        if let Some(id) = request.abort_id {
            self.state.borrow_mut().finish_fetch_cancellation(id);
        }
    }

    fn drain_body(&mut self, request: &mut PendingFetch, context: &mut Context) -> DrainOutcome {
        let outcome = self.drain_body_queue(request, context);
        if matches!(outcome, DrainOutcome::Finished) {
            self.finish_fetch_request(request, context);
        }
        outcome
    }

    fn drain_body_queue(&mut self, request: &mut PendingFetch, context: &mut Context) -> DrainOutcome {
        let Some(mut body) = request.body.take() else {
            self.finish_fetch_request(request, context);
            self.detach_abort_signal(request, context);
            return DrainOutcome::Finished;
        };
        match flush_body_queue(&mut body, context) {
            Ok(BodyFlush::Drained) => {}
            Ok(BodyFlush::Blocked) => {
                request.body = Some(body);
                return DrainOutcome::Pending;
            }
            Ok(BodyFlush::Cancelled) => {
                self.cancel_body_transfer(request, context);
                return DrainOutcome::Finished;
            }
            Err(error) => {
                self.report_task_error(error);
                return DrainOutcome::Finished;
            }
        }
        if body.end_pending {
            if let Err(error) = close_body_stream(&body, context) {
                self.report_task_error(error);
            }
            self.record_fetch_trace(
                request,
                Some(&body.url),
                Some(body.status),
                Some(body.received as usize),
                (body.status >= 400).then(|| format!("HTTP status {}", body.status)),
            );
            self.detach_abort_signal(request, context);
            return DrainOutcome::Finished;
        }
        if let Some(error_pending) = body.error_pending.take() {
            if let Err(error) = error_body_stream(&body, error_pending.reason, context) {
                self.report_task_error(error);
            }
            self.record_fetch_trace(
                request,
                Some(&body.url),
                Some(body.status),
                Some(body.received as usize),
                error_pending.trace,
            );
            self.detach_abort_signal(request, context);
            return DrainOutcome::Finished;
        }
        request.body = Some(body);
        DrainOutcome::Pending
    }

    fn cancel_body_transfer(&mut self, request: &mut PendingFetch, context: &mut Context) {
        let mut state = self.state.borrow_mut();
        if let Some(id) = request.abort_id {
            state.abort_fetch(id, JsValue::undefined());
            state.finish_fetch_cancellation(id);
        }
        state.record_resource_load(ResourceLoadRecord::new(
            ResourceLoadKind::Fetch,
            &request.method,
            &request.request_url,
            None,
            None,
            request.started_at.elapsed(),
            None,
            Some("aborted"),
        ));
        drop(state);
        self.detach_abort_signal(request, context);
    }

    fn record_response_cookies(&mut self, request: &PendingFetch, head: &StreamHead) {
        if !request.include_credentials {
            return;
        }
        let Ok(target_url) = RixUrl::parse(&head.url) else {
            return;
        };
        let mut state = self.state.borrow_mut();
        if head.set_cookies.is_empty() {
            if let Some(raw_cookie) = head.headers.get("set-cookie") {
                if let Some(cookie) =
                    rix_net::client::parse_cookie_header_for_url(raw_cookie, &target_url)
                {
                    state.record_cookie_update(cookie);
                }
            }
            return;
        }
        for raw_cookie in &head.set_cookies {
            if let Some(cookie) =
                rix_net::client::parse_cookie_header_for_url(raw_cookie, &target_url)
            {
                state.record_cookie_update(cookie);
            }
        }
    }

    fn detach_abort_signal(&mut self, request: &PendingFetch, context: &mut Context) {
        if let (Some(signal), Some(registration)) =
            (&request.abort_signal, &request.abort_registration)
        {
            if let Err(error) = remove_abort_algorithm(signal, registration, context) {
                self.report_task_error(error);
            }
        }
    }

    fn record_fetch_trace(
        &mut self,
        request: &PendingFetch,
        url: Option<&str>,
        status: Option<u16>,
        bytes: Option<usize>,
        error: Option<String>,
    ) {
        self.state
            .borrow_mut()
            .record_resource_load(ResourceLoadRecord::new(
                ResourceLoadKind::Fetch,
                &request.method,
                &request.request_url,
                url,
                status,
                request.started_at.elapsed(),
                bytes,
                error.as_deref(),
            ));
    }

    pub fn take_resource_loads(&mut self) -> Vec<ResourceLoadRecord> {
        self.state.borrow_mut().take_resource_loads()
    }

    pub fn take_resource_loads_truncated(&mut self) -> bool {
        self.state.borrow_mut().take_resource_loads_truncated()
    }

    fn fire_timer(timer: &Timer, context: &mut Context) -> JsResult<JsValue> {
        let this: JsValue = context.global_object().into();
        match &timer.callback {
            TimerCallback::Function(cb) => cb.call(&this, &timer.args, context),
            TimerCallback::Code(code) => {
                context.eval(boa_engine::Source::from_bytes(code.as_bytes()))
            }
        }
    }

    pub fn step(&mut self, context: &mut Context) -> usize {
        self.step_with_progress(context, |_| {})
    }

    pub fn step_with_progress(
        &mut self,
        context: &mut Context,
        mut phase_started: impl FnMut(&'static str),
    ) -> usize {
        if self.has_pending_microtasks() {
            self.state.borrow_mut().end_idle_period_early();
            phase_started("Promise/microtask checkpoint");
        }
        if !self.drain_microtasks(context) {
            return 0;
        }
        if !self.state.borrow().pending_fetches.is_empty() {
            phase_started("Fetch completion polling");
        }
        let completed_fetches = self.poll_fetches(context);
        if completed_fetches > 0 {
            self.state.borrow_mut().end_idle_period_early();
        }
        if completed_fetches > 0 {
            if self.has_pending_microtasks() {
                phase_started("Fetch Promise checkpoint");
            }
            if !self.drain_microtasks(context) {
                return completed_fetches;
            }
        }

        let mut tasks_count = 0;
        let mut timer_batch = 0;
        while timer_batch < 16 {
            let (timer, timed_out_idle_callback) = {
                let mut state = self.state.borrow_mut();
                let now = Instant::now();
                let timer = state
                    .timers
                    .values()
                    .filter(|timer| timer.fire_at <= now)
                    .min_by_key(|timer| (timer.fire_at, timer.id))
                    .cloned();
                let idle_timeout = state
                    .earliest_idle_timeout()
                    .filter(|timeout| *timeout <= now);
                let timeout_precedes_timer = match (&timer, idle_timeout) {
                    (Some(timer), Some(timeout)) => timeout < timer.fire_at,
                    (None, Some(_)) => true,
                    _ => false,
                };
                if timeout_precedes_timer {
                    (None, state.take_timed_out_idle_callback(now))
                } else if let Some(timer) = &timer {
                    state.end_idle_period_early();
                    if let Some(interval) = timer.interval {
                        if let Some(scheduled) = state.timers.get_mut(&timer.id) {
                            scheduled.fire_at = Instant::now() + interval;
                        }
                    } else {
                        state.timers.remove(&timer.id);
                    }
                    (Some(timer.clone()), None)
                } else {
                    (None, None)
                }
            };

            if timer.is_none() && timed_out_idle_callback.is_none() {
                break;
            }

            tasks_count += 1;
            timer_batch += 1;

            if let Some(timer) = timer {
                phase_started("timer callback");
                let timer_trace = is_worker_trace();
                let timer_started = Instant::now();
                if timer_trace {
                    let source = match &timer.callback {
                        TimerCallback::Function(callback) => JsValue::from(callback.clone())
                            .to_string(context)
                            .ok()
                            .map(|text| text.to_std_string_lossy())
                            .unwrap_or_default(),
                        TimerCallback::Code(code) => code.clone(),
                    };
                    eprintln!(
                        "[rix-timer] start id={} interval={:?} overdue={}ms src={:?}",
                        timer.id,
                        timer.interval,
                        Instant::now()
                            .saturating_duration_since(timer.fire_at)
                            .as_millis(),
                        source.chars().take(300).collect::<String>()
                    );
                }
                if let Err(error) = Self::fire_timer(&timer, context) {
                    self.report_task_error(error);
                }
                let timer_elapsed = timer_started.elapsed();
                if timer_trace {
                    eprintln!(
                        "[rix-timer] end id={} callback={}ms",
                        timer.id,
                        timer_elapsed.as_millis()
                    );
                }
                if self.has_pending_microtasks() {
                    phase_started("timer Promise checkpoint");
                }
                if !self.drain_microtasks(context) {
                    return tasks_count + completed_fetches;
                }
            } else if let Some(callback) = timed_out_idle_callback {
                phase_started("idle callback timeout");
                let idle_trace = is_worker_trace();
                if idle_trace {
                    eprintln!("[rix-timer] idle-timeout id={}", callback.id);
                }
                if let Err(error) = invoke_idle_callback(&callback, Instant::now(), true, context) {
                    self.report_task_error(error);
                }
                if idle_trace {
                    eprintln!("[rix-timer] idle-timeout end id={}", callback.id);
                }
                if self.has_pending_microtasks() {
                    phase_started("idle callback Promise checkpoint");
                }
                if !self.drain_microtasks(context) {
                    return tasks_count + completed_fetches;
                }
            }
        }

        if self.has_pending_microtasks() {
            phase_started("final Promise job checkpoint");
        }
        self.run_jobs(context);
        if self.has_pending_microtasks() {
            return tasks_count + completed_fetches;
        }
        let rejection_events_pending = self
            .state
            .borrow()
            .promise_rejections
            .as_ref()
            .is_some_and(|tracker| tracker.borrow().has_pending_events());
        if rejection_events_pending {
            phase_started("Promise rejection event dispatch");
        }
        self.dispatch_promise_rejection_events(context);
        if self.has_pending_microtasks() {
            return tasks_count + completed_fetches;
        }

        let rejection_events_pending = self
            .state
            .borrow()
            .promise_rejections
            .as_ref()
            .is_some_and(|tracker| tracker.borrow().has_pending_events());
        if !self.has_pending_microtasks()
            && !rejection_events_pending
        {
            let mut idle_run_count = 0;
            while idle_run_count < 16 {
                let idle_callback = {
                    let now = Instant::now();
                    let mut state = self.state.borrow_mut();
                    let timer_is_due = state.timers.values().any(|timer| timer.fire_at <= now);
                    let next_timer = state
                        .timers
                        .values()
                        .map(|timer| timer.fire_at)
                        .filter(|deadline| *deadline > now)
                        .min();
                    if timer_is_due {
                        None
                    } else {
                        state.take_idle_callback(now, next_timer)
                    }
                };
                let Some((callback, deadline)) = idle_callback else {
                    break;
                };
                phase_started("idle callback");
                let idle_started = Instant::now();
                if let Err(error) = invoke_idle_callback(&callback, deadline, false, context) {
                    self.report_task_error(error);
                }
                let idle_elapsed = idle_started.elapsed();
                if is_worker_trace() && idle_elapsed >= Duration::from_millis(50) {
                    eprintln!(
                        "[rix-idle-slow] id={} callback={}ms",
                        callback.id,
                        idle_elapsed.as_millis()
                    );
                }
                tasks_count += 1;
                idle_run_count += 1;
                if self.has_pending_microtasks() {
                    phase_started("idle callback Promise checkpoint");
                }
                if !self.drain_microtasks(context) {
                    return tasks_count + completed_fetches;
                }

                self.dispatch_promise_rejection_events(context);
                if Instant::now() >= deadline {
                    break;
                }
            }
        }

        tasks_count + completed_fetches
    }

    pub fn run_until_idle(&mut self, context: &mut Context, max_iterations: usize) {
        for _ in 0..max_iterations {
            let fired = self.step(context);
            if fired == 0 && !self.has_pending() {
                break;
            }
            if let Some(deadline) = self.next_deadline() {
                let wait = deadline.saturating_duration_since(Instant::now());
                if !wait.is_zero() {
                    std::thread::sleep(wait.min(Duration::from_millis(10)));
                }
            }
        }
        self.run_jobs(context);
    }
}
