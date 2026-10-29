use super::*;

impl BrowserShell {
    pub(super) fn poll_page_workers(&mut self) -> usize {
        let updates: Vec<_> = self
            .pages
            .iter()
            .filter_map(|(id, page)| {
                page.page_worker
                    .as_ref()
                    .map(|worker| (*id, page.image_generation, worker.borrow_mut().poll()))
            })
            .collect();
        let mut changed = 0;
        let mut status_changed = false;
        for (tab_id, generation, updates) in updates {
            let mut latest = None;
            for update in updates {
                if !self
                    .pages
                    .get(&tab_id)
                    .is_some_and(|page| page.image_generation == generation)
                {
                    break;
                }
                match update {
                    crate::page_worker::Update::Snapshot { html, state } => {
                        latest = Some((html, state))
                    }
                    crate::page_worker::Update::AnimationFrameRequested => {
                        if let Some(page) = self.pages.get_mut(&tab_id) {
                            page.worker_animation_frame_requested = true;
                            changed += 1;
                        }
                    }
                    crate::page_worker::Update::AnimationFrameComplete => {
                        if let Some(page) = self.pages.get_mut(&tab_id) {
                            page.worker_animation_frame_in_flight = false;
                            page.worker_animation_frame_ready_to_paint = true;
                            changed += 1;
                        }
                    }
                    crate::page_worker::Update::FocusChanged { focus } => {
                        if self.apply_worker_focus(tab_id, focus) {
                            changed += 1;
                        }
                    }
                    crate::page_worker::Update::ScrollRequested { x, y } => {
                        if let Some(page) = self.pages.get_mut(&tab_id) {
                            page.scroll_x = x;
                            page.scroll_y = y;
                            changed += 1;
                        }
                    }
                    crate::page_worker::Update::NavigationRequest { url, kind } => {
                        use rix_js::web_api::PageNavigationKind;
                        match kind {
                            PageNavigationKind::Reload => self.start_reload(tab_id),
                            PageNavigationKind::Replace => {
                                self.start_navigation_without_history(tab_id, &url)
                            }
                            PageNavigationKind::Assign => self.start_navigation(tab_id, &url),
                        }
                        changed += 1;
                    }
                    crate::page_worker::Update::HistoryChanged { url, kind, index } => {
                        let was_pending = self
                            .pages
                            .get(&tab_id)
                            .map(|page| page.pending_history_traversal)
                            .unwrap_or(false);
                        if was_pending {
                            let normalized = self.normalize_navigation_target(&url);
                            if let Some(page) = self.pages.get_mut(&tab_id) {
                                page.pending_history_traversal = false;
                                if page.back_stack.last() == Some(&normalized) {
                                    page.back_stack.pop();
                                    let previous = page.url.clone();
                                    if !previous.is_empty() && previous != normalized {
                                        page.forward_stack.push(previous);
                                    }
                                } else if page.forward_stack.last() == Some(&normalized) {
                                    page.forward_stack.pop();
                                    let previous = page.url.clone();
                                    if !previous.is_empty() && previous != normalized {
                                        page.back_stack.push(previous);
                                    }
                                }
                            }
                        }
                        if self.apply_page_history_change(tab_id, &url, kind, index) {
                            changed += 1;
                        }
                    }
                    crate::page_worker::Update::HistoryTraversalUnavailable { amount } => {
                        if amount < 0 {
                            let _ = self.start_go_back(tab_id);
                        } else if amount > 0 {
                            let _ = self.start_go_forward(tab_id);
                        }
                        changed += 1;
                    }
                    crate::page_worker::Update::Error { message } => {
                        if let Some(page) = self.pages.get_mut(&tab_id) {
                            page.worker_animation_frame_requested = false;
                            page.worker_animation_frame_in_flight = false;
                            page.worker_animation_frame_ready_to_paint = true;
                            let base = page.status_message.split(" · JS: ").next().unwrap_or("");
                            let brief: String = message
                                .lines()
                                .next()
                                .unwrap_or("Page runtime failed")
                                .chars()
                                .take(240)
                                .collect();
                            page.status_message = format!("{base} · JS: {brief}");
                            status_changed = true;
                        }
                        tracing::warn!(tab_id, %message, "Page runtime reported error");
                        changed += 1;
                    }
                    crate::page_worker::Update::Console { level, message } => {
                        if matches!(level.as_str(), "warn" | "error") {
                            tracing::warn!(tab_id, console_level = %level, %message, "Page console");
                        } else {
                            tracing::debug!(tab_id, console_level = %level, %message, "Page console");
                        }
                    }
                    crate::page_worker::Update::ResourceLoad { record } => {
                        if let Some(page) = self.pages.get_mut(&tab_id) {
                            retain_resource_record(page, record);
                        }
                    }
                    crate::page_worker::Update::DynamicScriptRequest { target, source_url } => {
                        self.schedule_dynamic_script(tab_id, generation, target, source_url);
                        changed += 1;
                    }
                    crate::page_worker::Update::ModuleLoadRequest {
                        request_id,
                        source_url,
                    } => {
                        self.schedule_module_load(tab_id, generation, request_id, source_url);
                        changed += 1;
                    }
                    crate::page_worker::Update::ResourceLoadTraceTruncated => {
                        if let Some(page) = self.pages.get_mut(&tab_id) {
                            page.resource_trace_truncated = true;
                        }
                    }
                    crate::page_worker::Update::CookieSet { cookie } => {
                        if let Some(storage) =
                            self.storage.as_ref().or_else(|| self.http_client.storage())
                        {
                            let _ = storage.set_cookies(std::iter::once(cookie));
                        }
                    }
                    crate::page_worker::Update::StorageMutated { mutation } => {
                        if let Some(storage) =
                            self.storage.as_ref().or_else(|| self.http_client.storage())
                        {
                            match mutation {
                                rix_js::web_api::StorageMutation::SetItem {
                                    origin,
                                    key,
                                    value,
                                } => {
                                    let _ = storage.set_local_storage(&origin, &key, &value);
                                }
                                rix_js::web_api::StorageMutation::RemoveItem { origin, key } => {
                                    let _ = storage.remove_local_storage(&origin, &key);
                                }
                                rix_js::web_api::StorageMutation::Clear { origin } => {
                                    let _ = storage.clear_local_storage(&origin);
                                }
                            }
                        }
                    }
                    crate::page_worker::Update::TextEdited {
                        id,
                        target,
                        focus_epoch,
                        value,
                        selection,
                    } => {
                        if let Some(pending) = self.worker_keys.remove(&(tab_id, id)) {
                            let current = self.pages.get(&tab_id).map(|page| {
                                (
                                    page.page_worker
                                        .as_ref()
                                        .map(|worker| worker.borrow().focus_epoch()),
                                    page.page_worker.as_ref().and_then(|worker| {
                                        page.focused_input.and_then(|node| {
                                            worker.borrow().node_token(&page.document, node)
                                        })
                                    }),
                                )
                            });
                            if current == Some((Some(pending.focus_epoch), Some(target)))
                                && focus_epoch == pending.focus_epoch
                            {
                                self.pages.get_mut(&tab_id).unwrap().input_selection = selection;
                                self.commit_page_input_value(tab_id, value, false);
                            }
                        }
                    }
                    crate::page_worker::Update::EventHandled {
                        id,
                        allowed,
                        target,
                        focus_epoch,
                    } => {
                        if let Some(action) = self.worker_defaults.remove(&(tab_id, id)) {
                            if allowed {
                                if let ClickAction::NavigateUrl(url) = action {
                                    self.start_navigation(tab_id, &url);
                                }
                            }
                        }
                        if let Some(pending) = self.worker_keys.remove(&(tab_id, id)) {
                            let current = self.pages.get(&tab_id).map(|page| {
                                (
                                    page.page_worker
                                        .as_ref()
                                        .map(|worker| worker.borrow().focus_epoch()),
                                    page.page_worker.as_ref().and_then(|worker| {
                                        page.focused_input.and_then(|node| {
                                            worker.borrow().node_token(&page.document, node)
                                        })
                                    }),
                                )
                            });
                            if allowed
                                && self.active_tab_id() == tab_id
                                && focus_epoch == pending.focus_epoch
                                && target == pending.target
                                && current == Some((Some(pending.focus_epoch), pending.target))
                            {
                                self.apply_page_key_default(&pending.input);
                            }
                        }
                    }
                    crate::page_worker::Update::Ready => changed += 1,
                    crate::page_worker::Update::MediaSamples {
                        session_id,
                        track_id,
                        samples,
                        codec,
                        extra_data,
                        sample_rate,
                        channels,
                    } => {
                        let engine = self.open_media_engine(tab_id, session_id, 1);
                        let sample_count = samples.len();
                        let mut sample_bytes = 0usize;
                        for mut sample in samples {
                            if sample.track_id == 0 {
                                sample.track_id = track_id;
                            }
                            sample_bytes += sample.data.len();
                            if let Err(error) = engine.push_sample(
                                sample,
                                &codec,
                                &extra_data,
                                sample_rate,
                                channels,
                            ) {
                                tracing::warn!(
                                    %error,
                                    session = session_id.0,
                                    track = track_id,
                                    codec = %codec,
                                    "media sample rejected"
                                );
                            }
                        }
                        tracing::debug!(
                            session = session_id.0,
                            track = track_id,
                            codec = %codec,
                            samples = sample_count,
                            bytes = sample_bytes,
                            "media samples forwarded"
                        );
                        changed += 1;
                    }
                    crate::page_worker::Update::MediaEngineOpened { session_id } => {
                        self.open_media_engine(tab_id, session_id, 1);
                        changed += 1;
                    }
                    crate::page_worker::Update::MediaPlayback { session_id, command } => {
                        let engine = self.open_media_engine(tab_id, session_id, 1);
                        tracing::debug!(
                            session = session_id.0,
                            ?command,
                            "media playback command received"
                        );
                        if let Err(error) =
                            engine.handle_command(command, std::time::Instant::now())
                        {
                            tracing::warn!(
                                %error,
                                session = session_id.0,
                                "media playback command failed"
                            );
                        }
                        changed += 1;
                    }
                    _ => {}
                }
            }
            if let Some((html, state)) = latest.filter(|_| {
                self.pages
                    .get(&tab_id)
                    .is_some_and(|page| page.image_generation == generation)
            }) {
                self.queue_worker_snapshot_layout(tab_id, html, state);
                changed += 1;
            }
        }
        if status_changed {
            self.notify_chrome_tabs_changed();
        }
        changed += self.poll_dynamic_script_completions();
        changed += self.poll_module_load_completions();
        changed
    }

    fn schedule_dynamic_script(
        &mut self,
        tab_id: u32,
        generation: u64,
        target: crate::page_worker::NodeToken,
        source_url: String,
    ) {
        let policy = self.navigation_resource_policy;
        let Some(page) = self
            .pages
            .get_mut(&tab_id)
            .filter(|page| page.image_generation == generation)
        else {
            return;
        };
        if !page.dynamic_script_attempts.insert(target) {
            return;
        }
        let resource_count = page
            .resource_trace
            .iter()
            .filter(|record| {
                matches!(
                    record.kind,
                    ResourceLoadKind::Script | ResourceLoadKind::Stylesheet
                )
            })
            .count()
            + self
                .pending_dynamic_scripts
                .iter()
                .filter(|(pending_tab, pending_generation, _)| {
                    *pending_tab == tab_id && *pending_generation == generation
                })
                .count();
        let failure = if resource_count >= policy.max_resources {
            Some("navigation resource count limit exceeded".to_string())
        } else if page.loaded_source_bytes >= policy.max_source_bytes {
            Some("navigation source budget exceeded".to_string())
        } else {
            None
        };
        let parsed_url = RixUrl::parse(&source_url).ok().filter(|url| {
            matches!(
                url.scheme(),
                rix_core::url::RixScheme::Http | rix_core::url::RixScheme::Https
            )
        });
        let failure = failure.or_else(|| {
            parsed_url
                .is_none()
                .then(|| "dynamic script URL must use HTTP or HTTPS".to_string())
        });
        let worker = page.page_worker.clone();
        if let Some(message) = failure {
            let record = ResourceLoadRecord::new(
                ResourceLoadKind::Script,
                "GET",
                &source_url,
                None,
                None,
                std::time::Duration::ZERO,
                None,
                Some(&message),
            );
            retain_resource_record(page, record);
            if let Some(worker) = worker {
                let accepted =
                    worker
                        .borrow()
                        .complete_script_load(target, source_url, Err(message.clone()));
                if !accepted {
                    tracing::warn!(tab_id, %message, "Page worker rejected dynamic script failure event");
                }
            }
            return;
        }
        let Some(target_url) = parsed_url else {
            return;
        };
        let Some(worker) = worker else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            let message = "no asynchronous runtime is available for dynamic scripts".to_string();
            retain_resource_record(
                page,
                ResourceLoadRecord::new(
                    ResourceLoadKind::Script,
                    "GET",
                    &source_url,
                    None,
                    None,
                    std::time::Duration::ZERO,
                    None,
                    Some(&message),
                ),
            );
            let accepted =
                worker
                    .borrow()
                    .complete_script_load(target, source_url, Err(message.clone()));
            if !accepted {
                tracing::warn!(tab_id, %message, "Page worker rejected dynamic script failure event");
            }
            return;
        };
        let key = (tab_id, generation, target);
        if !self.pending_dynamic_scripts.insert(key) {
            return;
        }
        let client = self.http_client.clone();
        let sender = self.dynamic_script_tx.clone();
        let semaphore = page.dynamic_script_semaphore.clone();
        let max_bytes = policy.max_script_bytes;
        let timeout = policy.resource_timeout;
        let request_url = source_url.clone();
        let tasks = self.dynamic_script_tasks.entry(tab_id).or_default();
        tasks.retain(|task| !task.is_finished());
        tasks.push(handle.spawn(async move {
            let permit = semaphore.acquire_owned().await;
            let (source, resource_record) = match permit {
                Ok(_permit) => {
                    load_dynamic_script(client, request_url.clone(), target_url, max_bytes, timeout)
                        .await
                }
                Err(error) => {
                    let message = error.to_string();
                    let record = ResourceLoadRecord::new(
                        ResourceLoadKind::Script,
                        "GET",
                        &request_url,
                        None,
                        None,
                        std::time::Duration::ZERO,
                        None,
                        Some(&message),
                    );
                    (Err(message), record)
                }
            };
            let _ = sender.send(DynamicScriptCompletion {
                tab_id,
                generation,
                target,
                source_url,
                source,
                resource_record,
            });
        }));
    }

    fn schedule_module_load(
        &mut self,
        tab_id: u32,
        generation: u64,
        request_id: u64,
        source_url: String,
    ) {
        let key = (tab_id, generation, request_id);
        if self.pending_module_loads.contains(&key) {
            return;
        }
        let cached = self
            .pages
            .get(&tab_id)
            .filter(|page| page.image_generation == generation)
            .and_then(|page| page.module_source_cache.get(&source_url).cloned());
        if let Some((final_url, source)) = cached {
            if let Some(worker) = self
                .pages
                .get(&tab_id)
                .filter(|page| page.image_generation == generation)
                .and_then(|page| page.page_worker.clone())
            {
                let accepted =
                    worker
                        .borrow()
                        .complete_module_load(request_id, final_url, Ok(source));
                if !accepted {
                    tracing::warn!(
                        tab_id,
                        request_id,
                        "Page worker rejected cached module source"
                    );
                }
            }
            return;
        }

        let policy = self.navigation_resource_policy;
        let Some(page) = self
            .pages
            .get_mut(&tab_id)
            .filter(|page| page.image_generation == generation)
        else {
            return;
        };
        let resource_count = page
            .resource_trace
            .iter()
            .filter(|record| {
                matches!(
                    record.kind,
                    ResourceLoadKind::Script | ResourceLoadKind::Stylesheet
                )
            })
            .count()
            + self
                .pending_dynamic_scripts
                .iter()
                .filter(|(pending_tab, pending_generation, _)| {
                    *pending_tab == tab_id && *pending_generation == generation
                })
                .count()
            + self
                .pending_module_loads
                .iter()
                .filter(|(pending_tab, pending_generation, _)| {
                    *pending_tab == tab_id && *pending_generation == generation
                })
                .count();
        let failure = if resource_count >= policy.max_resources {
            Some("navigation resource count limit exceeded".to_string())
        } else if page.loaded_source_bytes >= policy.max_source_bytes {
            Some("navigation source budget exceeded".to_string())
        } else {
            None
        };
        let parsed_url = RixUrl::parse(&source_url).ok().filter(|url| {
            matches!(
                url.scheme(),
                rix_core::url::RixScheme::Http | rix_core::url::RixScheme::Https
            )
        });
        let failure = failure.or_else(|| {
            parsed_url
                .is_none()
                .then(|| "module URL must use HTTP or HTTPS".to_string())
        });
        let worker = page.page_worker.clone();
        if let Some(message) = failure {
            retain_resource_record(
                page,
                ResourceLoadRecord::new(
                    ResourceLoadKind::Script,
                    "GET",
                    &source_url,
                    None,
                    None,
                    std::time::Duration::ZERO,
                    None,
                    Some(&message),
                ),
            );
            if let Some(worker) = worker {
                let accepted = worker.borrow().complete_module_load(
                    request_id,
                    source_url,
                    Err(message.clone()),
                );
                if !accepted {
                    tracing::warn!(tab_id, request_id, %message, "Page worker rejected module failure");
                }
            }
            return;
        }
        let Some(target_url) = parsed_url else {
            return;
        };
        let Some(worker) = worker else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            let message = "no asynchronous runtime is available for module loads".to_string();
            retain_resource_record(
                page,
                ResourceLoadRecord::new(
                    ResourceLoadKind::Script,
                    "GET",
                    &source_url,
                    None,
                    None,
                    std::time::Duration::ZERO,
                    None,
                    Some(&message),
                ),
            );
            let accepted =
                worker
                    .borrow()
                    .complete_module_load(request_id, source_url, Err(message.clone()));
            if !accepted {
                tracing::warn!(tab_id, request_id, %message, "Page worker rejected module failure");
            }
            return;
        };
        if !self.pending_module_loads.insert(key) {
            return;
        }
        let client = self.http_client.clone();
        let sender = self.module_load_tx.clone();
        let semaphore = page.dynamic_script_semaphore.clone();
        let max_bytes = policy.max_script_bytes;
        let timeout = policy.resource_timeout;
        let request_url = source_url;
        let tasks = self.module_load_tasks.entry(tab_id).or_default();
        tasks.retain(|task| !task.is_finished());
        tasks.push(handle.spawn(async move {
            let permit = semaphore.acquire_owned().await;
            let (result, resource_record) = match permit {
                Ok(_permit) => {
                    load_module_source(client, request_url.clone(), target_url, max_bytes, timeout)
                        .await
                }
                Err(error) => {
                    let message = error.to_string();
                    let record = ResourceLoadRecord::new(
                        ResourceLoadKind::Script,
                        "GET",
                        &request_url,
                        None,
                        None,
                        std::time::Duration::ZERO,
                        None,
                        Some(&message),
                    );
                    (Err(message), record)
                }
            };
            let _ = sender.send(ModuleLoadCompletion {
                tab_id,
                generation,
                request_id,
                request_url,
                result,
                resource_record,
            });
        }));
    }

    fn poll_dynamic_script_completions(&mut self) -> usize {
        let mut completed = 0;
        while let Ok(mut completion) = self.dynamic_script_rx.try_recv() {
            self.pending_dynamic_scripts.remove(&(
                completion.tab_id,
                completion.generation,
                completion.target,
            ));
            let Some(page) = self
                .pages
                .get_mut(&completion.tab_id)
                .filter(|page| page.image_generation == completion.generation)
            else {
                continue;
            };
            if let Ok(source) = &completion.source {
                if source.len()
                    > self
                        .navigation_resource_policy
                        .max_source_bytes
                        .saturating_sub(page.loaded_source_bytes)
                {
                    let message = "navigation source budget exceeded".to_string();
                    completion.resource_record.error = Some(message.clone());
                    completion.source = Err(message);
                } else {
                    page.loaded_source_bytes =
                        page.loaded_source_bytes.saturating_add(source.len());
                }
            }
            retain_resource_record(page, completion.resource_record);
            let worker = page.page_worker.clone();
            if let Some(worker) = worker {
                let accepted = worker.borrow().complete_script_load(
                    completion.target,
                    completion.source_url,
                    completion.source,
                );
                if !accepted {
                    tracing::warn!(
                        tab_id = completion.tab_id,
                        target = completion.target,
                        "Page worker rejected dynamic script result"
                    );
                }
            }
            completed += 1;
        }
        completed
    }

    fn poll_module_load_completions(&mut self) -> usize {
        let mut completed = 0;
        while let Ok(mut completion) = self.module_load_rx.try_recv() {
            self.pending_module_loads.remove(&(
                completion.tab_id,
                completion.generation,
                completion.request_id,
            ));
            let Some(page) = self
                .pages
                .get_mut(&completion.tab_id)
                .filter(|page| page.image_generation == completion.generation)
            else {
                continue;
            };
            let source_url = match &completion.result {
                Ok((source_url, source)) => {
                    if source.len()
                        > self
                            .navigation_resource_policy
                            .max_source_bytes
                            .saturating_sub(page.loaded_source_bytes)
                    {
                        let message = "navigation source budget exceeded".to_string();
                        completion.resource_record.error = Some(message.clone());
                        completion.result = Err(message);
                        completion.request_url.clone()
                    } else {
                        page.loaded_source_bytes =
                            page.loaded_source_bytes.saturating_add(source.len());
                        page.module_source_cache.insert(
                            completion.request_url.clone(),
                            (source_url.clone(), source.clone()),
                        );
                        page.module_source_cache
                            .insert(source_url.clone(), (source_url.clone(), source.clone()));
                        source_url.clone()
                    }
                }
                Err(_) => completion.request_url.clone(),
            };
            retain_resource_record(page, completion.resource_record);
            if let Some(worker) = page.page_worker.clone() {
                let result = completion.result.map(|(_, source)| source);
                let accepted =
                    worker
                        .borrow()
                        .complete_module_load(completion.request_id, source_url, result);
                if !accepted {
                    tracing::warn!(
                        tab_id = completion.tab_id,
                        request_id = completion.request_id,
                        "Page worker rejected module result"
                    );
                }
            }
            completed += 1;
        }
        completed
    }

    pub(super) fn apply_worker_focus(
        &mut self,
        tab_id: u32,
        focus: crate::page_worker::FocusState,
    ) -> bool {
        let Some(page) = self.pages.get_mut(&tab_id) else {
            return false;
        };
        let Some(worker) = page.page_worker.clone() else {
            return false;
        };
        let worker = worker.borrow();
        if focus.revision < worker.latest_revision() {
            return false;
        }
        worker.observe_revision(focus.revision);
        if focus.epoch != worker.focus_epoch() {
            return false;
        }
        let focused = match focus.target {
            Some(token) => match worker.resolve_token(&page.document, token) {
                Some(node) => Some(node),
                None => return false,
            },
            None => None,
        };
        drop(worker);
        let selection = focus.selection.unwrap_or_else(|| {
            if focused == page.focused_input {
                page.input_selection
            } else {
                TextSelection::collapsed(
                    focused
                        .and_then(|node| control_value(&page.document, node))
                        .map_or(0, |value| value.chars().count()),
                )
            }
        });
        if focused == page.focused_input && selection == page.input_selection {
            return false;
        }
        page.focused_input = focused;
        page.in_page_focused = focused.is_some();
        page.in_page_input = focused
            .and_then(|node| control_value(&page.document, node))
            .unwrap_or_default();
        page.input_selection = selection;
        self.synchronize_page_selection(tab_id);
        true
    }

    pub(super) fn queue_worker_snapshot_layout(
        &mut self,
        tab_id: u32,
        html: String,
        state: crate::page_worker::SnapshotState,
    ) {
        let Some(page) = self.pages.get_mut(&tab_id) else {
            return;
        };
        if page
            .page_worker
            .as_ref()
            .is_some_and(|worker| state.focus.revision < worker.borrow().latest_revision())
        {
            return;
        }
        if (!html.is_empty() && html.len() > self.navigation_resource_policy.max_document_bytes)
            || state.nodes.len() > MAX_DOCUMENT_NODES
        {
            return;
        }
        if self.pending_page_layouts.contains_key(&tab_id) {
            self.deferred_worker_snapshots.insert(
                tab_id,
                DeferredWorkerSnapshot {
                    generation: page.image_generation,
                    html,
                    state,
                },
            );
            return;
        }
        if page
            .page_worker
            .as_ref()
            .is_some_and(|worker| worker.borrow().snapshot_matches(&state))
        {
            self.pending_page_layouts.remove(&tab_id);
            self.apply_worker_focus(tab_id, state.focus);
            return;
        }
        let restore_started = std::time::Instant::now();
        let (document, nodes) = match state.document() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                page.status_message = error;
                return;
            }
        };
        if let Err(error) = validate_page_complexity(&document) {
            page.status_message = error;
            return;
        }
        let node_count = state.nodes.len();
        if std::env::var_os("RIX_WORKER_TRACE").is_some() {
            eprintln!(
                "[rix-shell] restore-snapshot {}ms nodes={} pid={}",
                restore_started.elapsed().as_millis(),
                node_count,
                std::process::id()
            );
        }
        page.node_count = node_count;
        page.document = document.clone();
        page.rendered_dom_revision = document.revision();
        let revision = document.revision();
        let url = page.url.clone();
        let sheets = page.external_stylesheets.clone();
        let fonts = self.font_database.clone();
        let (width, height) = page.layout_viewport;
        let request_id = next_page_display_revision();
        let sender = self.page_layout_tx.clone();
        self.pending_page_layouts.insert(tab_id, request_id);
        let token = self.layout_tokens.entry(tab_id).or_default().clone();
        token.store(request_id, std::sync::atomic::Ordering::Release);
        let compute = move || {
            if token.load(std::sync::atomic::Ordering::Acquire) != request_id {
                return PageLayoutCompletion {
                    tab_id,
                    revision,
                    request_id,
                    html,
                    document,
                    layout: rix_layout::tree::LayoutTree::new(),
                    layout_viewport: (width, height),
                    worker_snapshot: Some(InstalledWorkerSnapshot { state, nodes }),
                };
            }
            let layout = compute_page_layout(&document, &url, &sheets, width, height, &fonts);
            PageLayoutCompletion {
                tab_id,
                revision,
                request_id,
                html,
                document,
                layout,
                layout_viewport: (width, height),
                worker_snapshot: Some(InstalledWorkerSnapshot { state, nodes }),
            }
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(compute).await;
                let _ = sender.send(match result {
                    Ok(completion) => PageLayoutResult::Ready(completion),
                    Err(_) => PageLayoutResult::Failed { tab_id, request_id },
                });
            });
        } else {
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(compute));
                let _ = sender.send(match result {
                    Ok(completion) => PageLayoutResult::Ready(completion),
                    Err(_) => PageLayoutResult::Failed { tab_id, request_id },
                });
            });
        }
    }

    pub(super) fn schedule_page_images(&mut self) {
        let tab_id = self.active_tab_id();
        let Some(page) = self.pages.get_mut(&tab_id) else {
            return;
        };
        if page.is_loading && page.runtime_context.is_none() {
            return;
        }
        let doc_rev = page.document.revision();
        let scroll_y = page.scroll_y;
        let rev_changed = page.scheduled_images_revision != doc_rev;
        let scroll_changed = (scroll_y - page.scheduled_images_scroll_y).abs() > 200.0;
        if !rev_changed && !scroll_changed {
            return;
        }
        page.scheduled_images_revision = doc_rev;
        page.scheduled_images_scroll_y = scroll_y;
        let base = document_base_url(&page.document, &page.url);
        let viewport_height = page.layout_viewport.1;
        let lazy_threshold = scroll_y + viewport_height + 1200.0;

        let urls: Vec<_> = page
            .document
            .descendants_with_shadow_roots(page.document.root())
            .into_iter()
            .filter_map(|node| {
                let Some(NodeData::Element(element)) = page.document.get(node) else {
                    return None;
                };
                let is_lazy = element.get_attribute("loading") == Some("lazy");
                if is_lazy {
                    if let Some(box_data) = page.layout_tree.box_for_node(node) {
                        if box_data.dimensions.content.y > lazy_threshold {
                            return None;
                        }
                    }
                }
                let source = match element.name.local.as_str() {
                    "img" => rix_dom::select_image_source(
                        element.get_attribute("src"),
                        element.get_attribute("srcset"),
                    ),
                    "video" => element.get_attribute("poster"),
                    _ => None,
                }?;
                resolve_page_image(&base, source)
            })
            .collect();
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
            self.navigation_resource_policy.concurrent_resources.max(1),
        ));
        let tasks = self.image_tasks.entry(tab_id).or_default();
        tasks.retain(|task| !task.is_finished());
        for (url, target) in urls {
            if page.image_attempts.len() >= self.navigation_resource_policy.max_resources
                || !page.image_attempts.insert(url.clone())
            {
                continue;
            }
            let generation = page.image_generation;
            self.pending_images
                .insert((tab_id, generation, url.clone()));
            let client = self
                .http_client
                .clone()
                .with_max_response_bytes(MAX_RESOURCE_BYTES)
                .with_timeout(std::time::Duration::from_secs(10));
            let sender = self.image_tx.clone();
            let limit = limit.clone();
            tasks.push(handle.spawn(async move {
                let _permit = limit.acquire_owned().await.ok();
                let (image, resource_record) = load_page_image(client, url.clone(), target).await;
                let _ = sender.send(PageImageCompletion {
                    tab_id,
                    generation,
                    url,
                    image,
                    resource_record,
                });
            }));
        }
    }

    pub(super) fn poll_image_completions(&mut self) -> usize {
        let mut changed = std::collections::HashSet::new();
        while let Ok(completion) = self.image_rx.try_recv() {
            self.pending_images.remove(&(
                completion.tab_id,
                completion.generation,
                completion.url.clone(),
            ));
            let Some(page) = self
                .pages
                .get_mut(&completion.tab_id)
                .filter(|page| page.image_generation == completion.generation)
            else {
                continue;
            };
            retain_resource_record(page, completion.resource_record);
            if let Ok(image) = completion.image {
                let used: usize = page.images.values().map(ImageResource::byte_len).sum();
                if used + image.byte_len() <= MAX_PAGE_IMAGE_BYTES {
                    page.images.insert(completion.url, image);
                    changed.insert(completion.tab_id);
                }
            }
        }
        for tab_id in &changed {
            let page = self.pages.get_mut(tab_id).unwrap();
            apply_page_image_resources(page, &self.font_database);
            page.display_revision = next_page_display_revision();
        }
        changed.len()
    }

    pub fn has_pending_page_layouts(&self) -> bool {
        !self.pending_page_layouts.is_empty()
            || !self.pending_images.is_empty()
            || !self.pending_dynamic_scripts.is_empty()
    }

    pub fn has_active_workers(&self) -> bool {
        self.pages.values().any(|page| {
            page.is_loading
                || page
                    .page_worker
                    .as_ref()
                    .is_some_and(|worker| worker.borrow().is_running())
        })
    }

    pub fn next_event_loop_deadline(&self) -> Option<std::time::Instant> {
        let mut deadline = self.event_loop.next_deadline();
        let active_tab_id = self.priv_state.borrow().active_tab_id;
        if let Some(page_context) = self
            .pages
            .get(&active_tab_id)
            .and_then(|page| page.runtime_context.as_ref())
        {
            if let Ok(context) = page_context.try_borrow() {
                if let Some(page_deadline) = context.event_loop.next_deadline() {
                    deadline = Some(deadline.map_or(page_deadline, |old| old.min(page_deadline)));
                }
            }
        }
        if self.pages.values().any(|page| {
            page.page_worker
                .as_ref()
                .is_some_and(|worker| worker.borrow().is_running())
        }) {
            let poll = std::time::Instant::now() + std::time::Duration::from_millis(16);
            deadline = Some(deadline.map_or(poll, |deadline| deadline.min(poll)));
        }
        deadline
    }
}

static DECODED_IMAGE_CACHE: std::sync::OnceLock<std::sync::RwLock<HashMap<String, ImageResource>>> =
    std::sync::OnceLock::new();

fn get_cached_decoded_image(url: &str) -> Option<ImageResource> {
    let cache = DECODED_IMAGE_CACHE.get_or_init(|| std::sync::RwLock::new(HashMap::with_capacity(1024)));
    cache.read().ok()?.get(url).cloned()
}

fn put_cached_decoded_image(url: String, image: ImageResource) {
    let cache = DECODED_IMAGE_CACHE.get_or_init(|| std::sync::RwLock::new(HashMap::with_capacity(1024)));
    if let Ok(mut writer) = cache.write() {
        if writer.len() >= 4096 {
            let to_remove: Vec<String> = writer.keys().take(1024).cloned().collect();
            for key in to_remove {
                writer.remove(&key);
            }
        }
        writer.insert(url, image);
    }
}

async fn load_page_image(
    client: HttpClient,
    request_url: String,
    source: PageImageSource,
) -> (Result<ImageResource, String>, ResourceLoadRecord) {
    let started = std::time::Instant::now();
    if let Some(cached) = get_cached_decoded_image(&request_url) {
        let record = ResourceLoadRecord::new(
            ResourceLoadKind::Image,
            "GET",
            &request_url,
            Some(&request_url),
            Some(200),
            started.elapsed(),
            Some(cached.byte_len()),
            None,
        );
        return (Ok(cached), record);
    }
    match source {
        PageImageSource::Inline(source) => {
            let result = tokio::task::spawn_blocking(move || {
                ImageResource::decode_data_uri(&source, MAX_DECODED_IMAGE_BYTES)
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            if let Ok(ref decoded) = result {
                put_cached_decoded_image(request_url.clone(), decoded.clone());
            }
            let record = ResourceLoadRecord::new(
                ResourceLoadKind::Image,
                "GET",
                &request_url,
                Some(&request_url),
                Some(200),
                started.elapsed(),
                None,
                result.as_ref().err().map(String::as_str),
            );
            (result, record)
        }
        PageImageSource::Network(target) => {
            let response =
                tokio::time::timeout(std::time::Duration::from_secs(10), client.fetch(&target))
                    .await;
            match response {
                Ok(Ok(response)) => {
                    let response_url = response.final_url.as_str().to_string();
                    let status = response.status;
                    let response_bytes = response.body.len();
                    let image = if status < 400 && response_bytes <= MAX_RESOURCE_BYTES {
                        tokio::task::spawn_blocking(move || {
                            ImageResource::decode(&response.body, MAX_DECODED_IMAGE_BYTES)
                        })
                        .await
                        .unwrap_or_else(|error| Err(error.to_string()))
                    } else if status >= 400 {
                        Err(format!("HTTP status {status}"))
                    } else {
                        Err("Image response exceeded resource byte limit".to_string())
                    };
                    if let Ok(ref decoded) = image {
                        put_cached_decoded_image(request_url.clone(), decoded.clone());
                        if response_url != request_url {
                            put_cached_decoded_image(response_url.clone(), decoded.clone());
                        }
                    }
                    let error = image.as_ref().err().map(String::as_str);
                    let record = ResourceLoadRecord::new(
                        ResourceLoadKind::Image,
                        "GET",
                        &request_url,
                        Some(&response_url),
                        Some(status),
                        started.elapsed(),
                        Some(response_bytes),
                        error,
                    );
                    (image, record)
                }
                Ok(Err(error)) => {
                    let message = error.to_string();
                    let record = ResourceLoadRecord::new(
                        ResourceLoadKind::Image,
                        "GET",
                        &request_url,
                        None,
                        None,
                        started.elapsed(),
                        None,
                        Some(&message),
                    );
                    (Err(message), record)
                }
                Err(_) => {
                    let message = "image request timed out".to_string();
                    let record = ResourceLoadRecord::new(
                        ResourceLoadKind::Image,
                        "GET",
                        &request_url,
                        None,
                        None,
                        started.elapsed(),
                        None,
                        Some(&message),
                    );
                    (Err(message), record)
                }
            }
        }
    }
}

async fn load_dynamic_script(
    client: HttpClient,
    request_url: String,
    target: RixUrl,
    max_bytes: usize,
    timeout: std::time::Duration,
) -> (Result<String, String>, ResourceLoadRecord) {
    let started = std::time::Instant::now();
    let response = client
        .with_max_response_bytes(max_bytes)
        .with_timeout(timeout)
        .fetch(&target)
        .await;
    match response {
        Ok(response) => {
            let response_url = response.final_url.as_str().to_string();
            let status = response.status;
            let response_bytes = response.body.len();
            let source = if !(200..300).contains(&status) {
                Err(format!("HTTP status {status}"))
            } else if response_bytes > max_bytes {
                Err("script response exceeded resource byte limit".to_string())
            } else {
                Ok(String::from_utf8(response.body)
                    .unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into_owned()))
            };
            let error = source.as_ref().err().map(String::as_str);
            let record = ResourceLoadRecord::new(
                ResourceLoadKind::Script,
                "GET",
                &request_url,
                Some(&response_url),
                Some(status),
                started.elapsed(),
                Some(response_bytes),
                error,
            );
            (source, record)
        }
        Err(error) => {
            let message = error.to_string();
            let record = ResourceLoadRecord::new(
                ResourceLoadKind::Script,
                "GET",
                &request_url,
                None,
                None,
                started.elapsed(),
                None,
                Some(&message),
            );
            (Err(message), record)
        }
    }
}

async fn load_module_source(
    client: HttpClient,
    request_url: String,
    target: RixUrl,
    max_bytes: usize,
    timeout: std::time::Duration,
) -> (Result<(String, String), String>, ResourceLoadRecord) {
    let started = std::time::Instant::now();
    let response = client
        .with_max_response_bytes(max_bytes)
        .with_timeout(timeout)
        .fetch(&target)
        .await;
    match response {
        Ok(response) => {
            let response_url = response.final_url.as_str().to_string();
            let status = response.status;
            let response_bytes = response.body.len();
            let source = if !(200..300).contains(&status) {
                Err(format!("HTTP status {status}"))
            } else if response_bytes > max_bytes {
                Err("module response exceeded resource byte limit".to_string())
            } else {
                Ok((
                    response_url.clone(),
                    String::from_utf8(response.body)
                        .unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into_owned()),
                ))
            };
            let error = source.as_ref().err().map(String::as_str);
            let record = ResourceLoadRecord::new(
                ResourceLoadKind::Script,
                "GET",
                &request_url,
                Some(&response_url),
                Some(status),
                started.elapsed(),
                Some(response_bytes),
                error,
            );
            (source, record)
        }
        Err(error) => {
            let message = error.to_string();
            let record = ResourceLoadRecord::new(
                ResourceLoadKind::Script,
                "GET",
                &request_url,
                None,
                None,
                started.elapsed(),
                None,
                Some(&message),
            );
            (Err(message), record)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoded_image_cache_roundtrip() {
        let pixels = vec![255u8; 16];
        let image = ImageResource::from_rgba(2, 2, &pixels).unwrap();
        let test_url = "https://example.com/test_avatar.png".to_string();

        put_cached_decoded_image(test_url.clone(), image.clone());
        let cached = get_cached_decoded_image(&test_url);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().data.width, 2);
    }
}
