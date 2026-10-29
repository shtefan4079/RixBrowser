use super::*;

pub(super) fn extract_inline_scripts(
    document: &Document,
    document_url: &str,
) -> Vec<crate::page_worker::PageScript> {
    let mut scripts = Vec::new();
    let mut script_element_index = 0;
    for node_id in document.descendants(document.root()) {
        let Some(NodeData::Element(element)) = document.get(node_id) else {
            continue;
        };
        if &*element.name.local != "script" {
            continue;
        }
        let element_index = script_element_index;
        script_element_index += 1;
        if element.get_attribute("src").is_some() {
            continue;
        }
        let script_type = element.get_attribute("type").unwrap_or("");
        let kind = if crate::page_worker::is_classic_script_type(script_type) {
            crate::page_worker::PageScriptKind::InlineClassic
        } else if crate::page_worker::is_module_script_type(script_type) {
            crate::page_worker::PageScriptKind::InlineModule
        } else {
            continue;
        };
        let mut source = String::new();
        for child_id in document.children(node_id) {
            if let Some(NodeData::Text(text)) = document.get(child_id) {
                source.push_str(text);
            }
        }
        if !source.trim().is_empty() {
            scripts.push(crate::page_worker::PageScript {
                source,
                element_index,
                kind,
                source_url: document_url.to_string(),
            });
        }
    }
    scripts
}

pub(super) fn simple_navigation_content(
    url: &str,
    html: String,
    status: &str,
) -> NavigationContent {
    let document = parse_html(&html);
    NavigationContent {
        scripts: extract_inline_scripts(&document, url),
        html,
        document,
        prepared_layout: None,
        layout_viewport: None,
        final_url: url.to_string(),
        status_message: status.to_string(),
        stylesheets: HashMap::new(),
        failed_resources: 0,
        navigation_clock: None,
        resource_trace: Vec::new(),
    }
}

pub(super) async fn load_navigation_content(
    client: HttpClient,
    requested_url: &str,
    viewport_width: f32,
    viewport_height: f32,
    fonts: FontDatabase,
    policy: NavigationResourcePolicy,
    theme: &str,
) -> NavigationContent {
    load_navigation_content_progressive(
        client,
        requested_url,
        viewport_width,
        viewport_height,
        fonts,
        None,
        policy,
        theme,
    )
    .await
}

pub(super) async fn load_navigation_content_progressive(
    client: HttpClient,
    requested_url: &str,
    viewport_width: f32,
    viewport_height: f32,
    fonts: FontDatabase,
    document_tx: Option<tokio::sync::mpsc::UnboundedSender<NavigationContent>>,
    policy: NavigationResourcePolicy,
    theme: &str,
) -> NavigationContent {
    if requested_url == "rix://welcome"
        || requested_url == "about:blank"
        || requested_url.starts_with("rix://")
    {
        return simple_navigation_content(
            requested_url,
            "<!doctype html><html><head><title></title></head><body></body></html>".to_string(),
            "Internal page",
        );
    }

    let target = match RixUrl::parse(requested_url) {
        Ok(target)
            if matches!(
                target.scheme(),
                rix_core::url::RixScheme::Http | rix_core::url::RixScheme::Https
            ) =>
        {
            target
        }
        _ => {
            let html = error_page_html(
                requested_url,
                "RixBrowser can only open http and https addresses.",
                theme,
            );
            return simple_navigation_content(requested_url, html, "Unsupported URL");
        }
    };
    let document_started = std::time::Instant::now();
    let mut preconnected_hosts = std::collections::HashSet::<String>::new();
    let mut prefetched_hosts = std::collections::HashSet::<String>::new();
    let mut preloaded_targets = std::collections::HashSet::<String>::new();

    let stream_res = client
        .clone()
        .with_max_response_bytes(policy.max_document_bytes)
        .fetch_stream(&target)
        .await;

    let response = match stream_res {
        Ok(mut streaming) => {
            let mut full_body = Vec::new();
            let mut scanner = super::preload_scanner::PreloadScanner::new(&streaming.final_url);
            let preload_client = client.clone();
            let mut stream_error = None;

            while let Some(chunk_res) = streaming.body.next_chunk().await {
                match chunk_res {
                    Ok(chunk) => {
                        if full_body.len() + chunk.len() > policy.max_document_bytes {
                            break;
                        }
                        scanner.scan_chunk(&chunk, |hint| match hint {
                            super::preload_scanner::PreloadHint::Preconnect(target) => {
                                if let Some(host) = target.host_str() {
                                    if preconnected_hosts.insert(host.to_string()) {
                                        let c = preload_client.clone();
                                        tokio::spawn(async move {
                                            c.preconnect(&target).await;
                                        });
                                    }
                                }
                            }
                            super::preload_scanner::PreloadHint::DnsPrefetch(host) => {
                                if !host.is_empty()
                                    && prefetched_hosts.insert(host.clone())
                                    && !preconnected_hosts.contains(&host)
                                {
                                    let c = preload_client.clone();
                                    tokio::spawn(async move {
                                        c.dns_prefetch(&host).await;
                                    });
                                }
                            }
                            super::preload_scanner::PreloadHint::FetchStylesheet(target) => {
                                if preloaded_targets.insert(target.as_str().to_string()) {
                                    let c = preload_client
                                        .clone()
                                        .with_max_response_bytes(policy.max_stylesheet_bytes)
                                        .with_timeout(policy.resource_timeout);
                                    tokio::spawn(async move {
                                        let _ = c.fetch(&target).await;
                                    });
                                }
                            }
                            super::preload_scanner::PreloadHint::FetchScript(target) => {
                                if preloaded_targets.insert(target.as_str().to_string()) {
                                    let c = preload_client
                                        .clone()
                                        .with_max_response_bytes(policy.max_script_bytes)
                                        .with_timeout(policy.resource_timeout);
                                    tokio::spawn(async move {
                                        let _ = c.fetch(&target).await;
                                    });
                                }
                            }
                        });
                        full_body.extend_from_slice(&chunk);
                    }
                    Err(e) => {
                        stream_error = Some(e);
                        break;
                    }
                }
            }

            if let Some(err) = stream_error {
                if full_body.is_empty() {
                    let mut content = simple_navigation_content(
                        requested_url,
                        error_page_html(requested_url, &err.to_string(), theme),
                        "Network error",
                    );
                    content.resource_trace.push(ResourceLoadRecord::new(
                        ResourceLoadKind::Document,
                        "GET",
                        requested_url,
                        None,
                        None,
                        document_started.elapsed(),
                        None,
                        Some(&err.to_string()),
                    ));
                    return content;
                }
            }

            let resp = rix_net::client::HttpResponse {
                status: streaming.status,
                headers: streaming.headers,
                body: full_body,
                final_url: streaming.final_url,
                content_type: streaming.content_type,
                protocol: streaming.protocol,
                navigation_timing: streaming.navigation_timing,
                navigation_started: streaming.navigation_started,
                set_cookies: streaming.set_cookies,
            };
            client.cache_response(requested_url, &resp);
            resp
        }
        Err(error) => {
            let mut content = simple_navigation_content(
                requested_url,
                error_page_html(requested_url, &error.to_string(), theme),
                "Network error",
            );
            content.resource_trace.push(ResourceLoadRecord::new(
                ResourceLoadKind::Document,
                "GET",
                requested_url,
                None,
                None,
                document_started.elapsed(),
                None,
                Some(&error.to_string()),
            ));
            return content;
        }
    };
    let mut resource_trace = vec![ResourceLoadRecord::new(
        ResourceLoadKind::Document,
        "GET",
        requested_url,
        Some(response.final_url.as_str()),
        Some(response.status),
        document_started.elapsed(),
        Some(response.body.len()),
        (response.status >= 400)
            .then(|| format!("HTTP status {}", response.status))
            .as_deref(),
    )];
    let navigation_clock = Some((response.navigation_timing, response.navigation_started));
    let final_url = response.final_url.as_str().to_string();
    let status_message = format!("HTTP {}", response.status);
    if std::env::var_os("RIX_WORKER_TRACE").is_some() {
        eprintln!(
            "[rix-document-timing] status={} body_bytes={} fetch_start={:.1}ms request_start={:.1}ms response_start(TTFB)={:.1}ms response_end={:.1}ms total={:.1}ms url={}",
            response.status,
            response.body.len(),
            response.navigation_timing.fetch_start_ms,
            response.navigation_timing.request_start_ms,
            response.navigation_timing.response_start_ms,
            response.navigation_timing.response_end_ms,
            document_started.elapsed().as_secs_f64() * 1000.0,
            final_url
        );
    }
    if response.body.len() > policy.max_document_bytes {
        let error = format!(
            "Document is larger than the {} MiB limit",
            policy.max_document_bytes / (1024 * 1024)
        );
        let mut content = simple_navigation_content(
            &final_url,
            error_page_html(&final_url, &error, theme),
            "Document too large",
        );
        content.resource_trace = resource_trace;
        return content;
    }
    let html = String::from_utf8(response.body)
        .unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into_owned());
    let t_parse = std::time::Instant::now();
    let mut document = parse_html(&html);
    let parse_ms = t_parse.elapsed().as_secs_f64() * 1000.0;
    if std::env::var_os("RIX_WORKER_TRACE").is_some() {
        eprintln!(
            "[rix-html-parse] parse_html took {:.2}ms (nodes={})",
            parse_ms,
            document.node_count()
        );
    }
    if final_url.contains("youtube.com") || final_url.contains("youtu.be") {
        if let Some(html_id) = document.document_element() {
            if let Some(NodeData::Element(elem)) = document.get_mut(html_id) {
                if theme == "dark" {
                    elem.set_attribute("dark", "");
                } else {
                    elem.remove_attribute("dark");
                }
            }
        }
    }
    if let Err(error) = validate_page_complexity(&document) {
        let mut content = simple_navigation_content(
            &final_url,
            error_page_html(&final_url, &error, theme),
            "Document too complex",
        );
        content.resource_trace = resource_trace;
        return content;
    }
    let base_url = document_base_url(&document, &final_url);
    let mut script_sources = Vec::<ScriptSource>::new();
    let mut script_element_index = 0;
    let mut jobs = Vec::<(ExternalResourceKind, String, RixUrl)>::new();
    let mut unique_jobs = std::collections::HashSet::<(u8, String)>::new();
    let mut failed_resources = 0;
    let mut source_budget = SourceBudget::new(policy.max_source_bytes);

    for node_id in document.descendants(document.root()) {
        let Some(NodeData::Element(element)) = document.get(node_id) else {
            continue;
        };
        let tag = &*element.name.local;
        if tag == "link" {
            let rel = element.get_attribute("rel").unwrap_or("");
            let rel_tokens: Vec<&str> = rel.split_ascii_whitespace().collect();

            if rel_tokens.iter().any(|token| token.eq_ignore_ascii_case("preconnect")) {
                if let Some(href) = element.get_attribute("href") {
                    if let Some((_, target)) = resolve_external_resource(&base_url, href) {
                        if let Some(host) = target.host_str() {
                            if preconnected_hosts.insert(host.to_string()) {
                                let preconnect_client = client.clone();
                                tokio::spawn(async move {
                                    preconnect_client.preconnect(&target).await;
                                });
                            }
                        }
                    }
                }
            }

            if rel_tokens.iter().any(|token| token.eq_ignore_ascii_case("dns-prefetch")) {
                if let Some(href) = element.get_attribute("href") {
                    let host = if href.starts_with("//") {
                        href.trim_start_matches('/').split('/').next().unwrap_or("").to_string()
                    } else if let Some((_, target)) = resolve_external_resource(&base_url, href) {
                        target.host_str().unwrap_or("").to_string()
                    } else {
                        String::new()
                    };
                    if !host.is_empty() && prefetched_hosts.insert(host.clone()) && !preconnected_hosts.contains(&host) {
                        let prefetch_client = client.clone();
                        tokio::spawn(async move {
                            prefetch_client.dns_prefetch(&host).await;
                        });
                    }
                }
            }

            if rel_tokens.iter().any(|token| token.eq_ignore_ascii_case("preload")) {
                let as_attr = element.get_attribute("as").unwrap_or("");
                if let Some(href) = element.get_attribute("href") {
                    if let Some((key, target)) = resolve_external_resource(&base_url, href) {
                        if as_attr.eq_ignore_ascii_case("style") {
                            if unique_jobs.insert((0, key.clone())) {
                                if jobs.len() < policy.max_resources {
                                    jobs.push((ExternalResourceKind::Stylesheet, key, target));
                                } else {
                                    failed_resources += 1;
                                }
                            }
                        } else if as_attr.eq_ignore_ascii_case("script") {
                            if unique_jobs.insert((1, key.clone())) {
                                if jobs.len() < policy.max_resources {
                                    jobs.push((ExternalResourceKind::Script, key, target));
                                } else {
                                    failed_resources += 1;
                                }
                            }
                        }
                    }
                }
            }

            if rel_tokens.iter().any(|token| token.eq_ignore_ascii_case("stylesheet")) {
                if let Some(href) = element.get_attribute("href") {
                    match resolve_external_resource(&base_url, href) {
                        Some((key, target)) if unique_jobs.insert((0, key.clone())) => {
                            if jobs.len() < policy.max_resources {
                                jobs.push((ExternalResourceKind::Stylesheet, key, target));
                            } else {
                                failed_resources += 1;
                            }
                        }
                        Some(_) => {}
                        None => failed_resources += 1,
                    }
                }
            }
        } else if tag == "script" {
            let element_index = script_element_index;
            script_element_index += 1;
            let script_type = element.get_attribute("type").unwrap_or("");
            let module = crate::page_worker::is_module_script_type(script_type);
            if !module && !crate::page_worker::is_classic_script_type(script_type) {
                continue;
            }
            if let Some(src) = element.get_attribute("src") {
                match resolve_external_resource(&base_url, src) {
                    Some((key, target)) => {
                        script_sources.push(ScriptSource::External {
                            key: key.clone(),
                            element_index,
                            module,
                        });
                        let job_kind = if module {
                            ExternalResourceKind::Module
                        } else {
                            ExternalResourceKind::Script
                        };
                        let unique_kind = if module { 2 } else { 1 };
                        if unique_jobs.insert((unique_kind, key.clone())) {
                            if jobs.len() < policy.max_resources {
                                jobs.push((job_kind, key, target));
                            } else {
                                failed_resources += 1;
                            }
                        }
                    }
                    None => failed_resources += 1,
                }
            } else {
                let mut source = String::new();
                for child_id in document.children(node_id) {
                    if let Some(NodeData::Text(text)) = document.get(child_id) {
                        source.push_str(text);
                    }
                }
                if !source.trim().is_empty() {
                    if source.len() <= policy.max_script_bytes && source_budget.retain(source.len())
                    {
                        script_sources.push(ScriptSource::Inline {
                            source,
                            element_index,
                            module,
                        });
                    } else {
                        failed_resources += 1;
                    }
                }
            }
        }
    }

    let mut tasks = tokio::task::JoinSet::new();
    let mut pending_resource_traces = HashMap::<String, (ResourceLoadKind, String)>::new();
    let resource_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
        policy.concurrent_resources.max(1),
    ));
    for (kind, key, target) in jobs {
        let trace_kind = match kind {
            ExternalResourceKind::Stylesheet => ResourceLoadKind::Stylesheet,
            ExternalResourceKind::Script | ExternalResourceKind::Module => ResourceLoadKind::Script,
        };
        let trace_key = external_resource_trace_key(kind, &key);
        let request_url = target.as_str().to_string();
        pending_resource_traces.insert(trace_key, (trace_kind, request_url.clone()));
        let max_size = match kind {
            ExternalResourceKind::Stylesheet => policy.max_stylesheet_bytes,
            ExternalResourceKind::Script | ExternalResourceKind::Module => policy.max_script_bytes,
        };
        let client = client
            .clone()
            .with_max_response_bytes(max_size)
            .with_timeout(policy.resource_timeout);
        let resource_limit = resource_limit.clone();
        tasks.spawn(async move {
            let started = std::time::Instant::now();
            let _permit = resource_limit.acquire_owned().await.ok();
            let result = client.fetch(&target).await;
            (kind, key, request_url, started.elapsed(), result)
        });
    }
    let resource_deadline = tokio::time::Instant::now() + policy.resource_timeout;

    if let Some(sender) = document_tx {
        let initial_document = document.clone();
        let initial_url = final_url.clone();
        let initial_fonts = fonts.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            let layout = compute_page_layout(
                &initial_document,
                &initial_url,
                &HashMap::new(),
                viewport_width.max(1.0),
                viewport_height.max(1.0),
                &initial_fonts,
            );
            (initial_document, layout)
        })
        .await;
        if let Ok((document, layout)) = prepared {
            let _ = sender.send(NavigationContent {
                html: html.clone(),
                document,
                prepared_layout: Some(layout),
                layout_viewport: Some((viewport_width.to_bits(), viewport_height.to_bits())),
                final_url: final_url.clone(),
                status_message: status_message.clone(),
                stylesheets: HashMap::new(),
                scripts: Vec::new(),
                failed_resources: 0,
                navigation_clock,
                resource_trace: resource_trace.clone(),
            });
        }
    }
    let mut stylesheets = HashMap::new();
    let mut external_scripts = HashMap::new();
    let mut external_modules = HashMap::<String, (String, String)>::new();
    while !tasks.is_empty() {
        let result = match tokio::time::timeout_at(resource_deadline, tasks.join_next()).await {
            Ok(Some(result)) => result,
            Ok(None) => break,
            Err(_) => {
                failed_resources += tasks.len();
                for (_, (kind, url)) in pending_resource_traces.drain() {
                    resource_trace.push(ResourceLoadRecord::new(
                        kind,
                        "GET",
                        &url,
                        None,
                        None,
                        policy.resource_timeout,
                        None,
                        Some("resource deadline exceeded"),
                    ));
                }
                tasks.abort_all();
                break;
            }
        };
        match result {
            Ok((kind, key, request_url, elapsed, Ok(resource))) => {
                pending_resource_traces.remove(&external_resource_trace_key(kind, &key));
                let trace_kind = match kind {
                    ExternalResourceKind::Stylesheet => ResourceLoadKind::Stylesheet,
                    ExternalResourceKind::Script | ExternalResourceKind::Module => {
                        ResourceLoadKind::Script
                    }
                };
                let max_size = match kind {
                    ExternalResourceKind::Stylesheet => policy.max_stylesheet_bytes,
                    ExternalResourceKind::Script | ExternalResourceKind::Module => {
                        policy.max_script_bytes
                    }
                };
                let response_bytes = resource.body.len();
                let response_url = resource.final_url.as_str().to_string();
                let mut failure = if !(200..300).contains(&resource.status) {
                    Some(format!("HTTP status {}", resource.status))
                } else if response_bytes > max_size {
                    Some("response exceeded resource byte limit".to_string())
                } else {
                    None
                };
                if failure.is_none() {
                    let source = String::from_utf8(resource.body)
                        .unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into_owned());
                    if !source_budget.retain(source.len()) {
                        tracing::warn!(%key, "Navigation source budget exceeded");
                        failure = Some("navigation source budget exceeded".to_string());
                    } else {
                        match kind {
                            ExternalResourceKind::Stylesheet => {
                                stylesheets.insert(key, source);
                            }
                            ExternalResourceKind::Script => {
                                external_scripts.insert(key, source);
                            }
                            ExternalResourceKind::Module => {
                                external_modules.insert(key, (source, response_url.clone()));
                            }
                        }
                    }
                }
                resource_trace.push(ResourceLoadRecord::new(
                    trace_kind,
                    "GET",
                    &request_url,
                    Some(&response_url),
                    Some(resource.status),
                    elapsed,
                    Some(response_bytes),
                    failure.as_deref(),
                ));
                if failure.is_some() {
                    failed_resources += 1;
                }
            }
            Ok((kind, key, request_url, elapsed, Err(error))) => {
                pending_resource_traces.remove(&external_resource_trace_key(kind, &key));
                tracing::warn!(%key, %error, "Navigation resource fetch failed");
                let trace_kind = match kind {
                    ExternalResourceKind::Stylesheet => ResourceLoadKind::Stylesheet,
                    ExternalResourceKind::Script | ExternalResourceKind::Module => {
                        ResourceLoadKind::Script
                    }
                };
                resource_trace.push(ResourceLoadRecord::new(
                    trace_kind,
                    "GET",
                    &request_url,
                    None,
                    None,
                    elapsed,
                    None,
                    Some(&error.to_string()),
                ));
                failed_resources += 1;
            }
            Err(error) => {
                tracing::warn!(%error, "Navigation resource task failed");
                failed_resources += 1;
            }
        }
    }
    let stylesheet_bytes: usize = stylesheets.values().map(String::len).sum();
    let mut executable_budget =
        SourceBudget::new(policy.max_source_bytes.saturating_sub(stylesheet_bytes));
    let mut scripts = Vec::<crate::page_worker::PageScript>::new();
    for source in script_sources {
        match source {
            ScriptSource::Inline {
                source,
                element_index,
                module,
            } => {
                if executable_budget.retain(source.len()) {
                    scripts.push(crate::page_worker::PageScript {
                        source,
                        element_index,
                        kind: if module {
                            crate::page_worker::PageScriptKind::InlineModule
                        } else {
                            crate::page_worker::PageScriptKind::InlineClassic
                        },
                        source_url: if module {
                            base_url.clone()
                        } else {
                            final_url.clone()
                        },
                    });
                } else {
                    failed_resources += 1;
                }
            }
            ScriptSource::External {
                key,
                element_index,
                module,
            } => {
                let source = if module {
                    external_modules
                        .get(&key)
                        .map(|(source, source_url)| (source, source_url.as_str()))
                } else {
                    external_scripts
                        .get(&key)
                        .map(|source| (source, key.as_str()))
                };
                if let Some((source, source_url)) = source {
                    if executable_budget.retain(source.len()) {
                        scripts.push(crate::page_worker::PageScript {
                            source: source.clone(),
                            element_index,
                            kind: if module {
                                crate::page_worker::PageScriptKind::ExternalModule
                            } else {
                                crate::page_worker::PageScriptKind::ExternalClassic
                            },
                            source_url: source_url.to_string(),
                        });
                    } else {
                        tracing::warn!(%key, "Executable source budget exceeded");
                        failed_resources += 1;
                    }
                }
            }
        }
    }
    let layout_url = final_url.clone();
    let layout_width = viewport_width.max(1.0);
    let layout_height = viewport_height.max(1.0);
    let prepared = tokio::task::spawn_blocking(move || {
        let layout = compute_page_layout(
            &document,
            &layout_url,
            &stylesheets,
            layout_width,
            layout_height,
            &fonts,
        );
        (document, stylesheets, layout)
    })
    .await;
    let (document, stylesheets, prepared_layout) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            let mut content = simple_navigation_content(
                &final_url,
                error_page_html(
                    &final_url,
                    &format!("Page layout worker failed: {error}"),
                    theme,
                ),
                "Layout error",
            );
            content.resource_trace = resource_trace;
            return content;
        }
    };
    NavigationContent {
        html,
        document,
        prepared_layout: Some(prepared_layout),
        layout_viewport: Some((viewport_width.to_bits(), viewport_height.to_bits())),
        final_url,
        status_message,
        stylesheets,
        scripts,
        failed_resources,
        navigation_clock,
        resource_trace,
    }
}

fn external_resource_trace_key(kind: ExternalResourceKind, key: &str) -> String {
    let prefix = match kind {
        ExternalResourceKind::Stylesheet => "stylesheet",
        ExternalResourceKind::Script => "script",
        ExternalResourceKind::Module => "module",
    };
    format!("{prefix}:{key}")
}

pub(super) fn validate_page_complexity(document: &Document) -> Result<(), String> {
    let mut pending = vec![(document.root(), 0)];
    let mut count = 0;
    while let Some((node, depth)) = pending.pop() {
        count += 1;
        if count > MAX_DOCUMENT_NODES {
            return Err(format!(
                "Document exceeds the {MAX_DOCUMENT_NODES} node limit"
            ));
        }
        if depth > MAX_DOCUMENT_DEPTH {
            return Err(format!(
                "Document exceeds the {MAX_DOCUMENT_DEPTH} nesting limit"
            ));
        }
        pending.extend(document.children(node).map(|child| (child, depth + 1)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_resource_hints_and_preloading() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    while let Ok(n) = socket.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        let req = String::from_utf8_lossy(&buf[..n]);
                        if req.starts_with("GET / ") {
                            let html = format!(
                                "<!doctype html><html><head>\
                                <link rel=\"preconnect\" href=\"http://127.0.0.1:{port}/\">\
                                <link rel=\"dns-prefetch\" href=\"//127.0.0.1\">\
                                <link rel=\"preload\" href=\"http://127.0.0.1:{port}/app.js\" as=\"script\">\
                                <link rel=\"stylesheet\" href=\"http://127.0.0.1:{port}/main.css\">\
                                <script src=\"http://127.0.0.1:{port}/app.js\"></script>\
                                </head><body><h1>Speed Test</h1></body></html>"
                            );
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{html}",
                                html.len()
                            );
                            let _ = socket.write_all(resp.as_bytes()).await;
                        } else if req.contains("GET /main.css ") {
                            let css = "body { background: white; }";
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{css}",
                                css.len()
                            );
                            let _ = socket.write_all(resp.as_bytes()).await;
                        } else if req.contains("GET /app.js ") {
                            let js = "console.log('loaded');";
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{js}",
                                js.len()
                            );
                            let _ = socket.write_all(resp.as_bytes()).await;
                        }
                    }
                });
            }
        });

        let client = HttpClient::new(None).unwrap();
        let fonts = FontDatabase::new();
        let target_url = format!("http://127.0.0.1:{port}/");
        let content = load_navigation_content(
            client,
            &target_url,
            1280.0,
            800.0,
            fonts,
            NavigationResourcePolicy::default(),
            "light",
        )
        .await;

        assert_eq!(content.failed_resources, 0);
        assert!(!content.stylesheets.is_empty());
        assert!(!content.scripts.is_empty());
    }
}
