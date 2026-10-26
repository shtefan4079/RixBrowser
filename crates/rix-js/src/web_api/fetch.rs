use super::*;
use boa_engine::builtins::promise::PromiseState;
use boa_engine::object::builtins::{JsArrayBuffer, JsDataView, JsTypedArray};

pub(super) fn register_fetch(context: &mut Context) -> Result<(), JsError> {
    context.register_global_callable(
        js_string!("fetch"),
        1,
        NativeFunction::from_fn_ptr(fetch_fn),
    )?;
    Ok(())
}

pub(super) fn percent_decode(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

pub(super) fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub(super) fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    for ch in input.chars() {
        if ch == '=' {
            break;
        }
        if ch.is_whitespace() {
            continue;
        }
        let v = match ch {
            'A'..='Z' => (ch as u8 - b'A') as u32,
            'a'..='z' => (ch as u8 - b'a' + 26) as u32,
            '0'..='9' => (ch as u8 - b'0' + 52) as u32,
            '+' => 62,
            '/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedFetch {
    pub(super) status: u16,
    pub(super) status_text: String,
    pub(super) body: Vec<u8>,
    pub(super) url: String,
    pub(super) headers: HashMap<String, String>,
    pub(super) redirected: bool,
}

pub(super) fn resolve_local_fetch(request_url: &str) -> Result<Option<ResolvedFetch>, String> {
    let absolute = request_url.to_string();
    if let Some(path) = absolute.strip_prefix("rix://chrome/") {
        if let Some((asset_bytes, mime)) =
            rix_net::resource::ResourceManager::get_embedded_chrome_asset(path)
        {
            return Ok(Some(ResolvedFetch {
                status: 200,
                status_text: "OK".to_string(),
                body: asset_bytes.to_vec(),
                url: absolute,
                headers: HashMap::from([("content-type".to_string(), mime.to_string())]),
                redirected: false,
            }));
        }
        if let Some((asset_bytes, mime)) =
            rix_net::resource::ResourceManager::get_embedded_chrome_asset(
                path.trim_start_matches('/'),
            )
        {
            return Ok(Some(ResolvedFetch {
                status: 200,
                status_text: "OK".to_string(),
                body: asset_bytes.to_vec(),
                url: absolute,
                headers: HashMap::from([("content-type".to_string(), mime.to_string())]),
                redirected: false,
            }));
        }
        return Ok(Some(ResolvedFetch {
            status: 404,
            status_text: "Not Found".to_string(),
            body: b"Not Found".to_vec(),
            url: absolute,
            headers: HashMap::from([("content-type".to_string(), "text/plain".to_string())]),
            redirected: false,
        }));
    }
    if absolute.starts_with("rix://") {
        return Ok(Some(ResolvedFetch {
            status: 404,
            status_text: "Not Found".to_string(),
            body: b"Not Found".to_vec(),
            url: absolute,
            headers: HashMap::from([("content-type".to_string(), "text/plain".to_string())]),
            redirected: false,
        }));
    }

    if let Some(rest) = absolute.strip_prefix("data:") {
        if let Some((meta, data_part)) = rest.split_once(',') {
            let is_base64 = meta.trim().ends_with(";base64");
            let mime = if is_base64 {
                meta.trim().trim_end_matches(";base64").to_string()
            } else {
                meta.to_string()
            };
            let content_type = if mime.is_empty() {
                "text/plain;charset=US-ASCII".to_string()
            } else {
                mime
            };
            let body = if is_base64 {
                base64_decode(data_part.trim()).unwrap_or_else(|| percent_decode(data_part))
            } else {
                percent_decode(data_part)
            };
            return Ok(Some(ResolvedFetch {
                status: 200,
                status_text: "OK".to_string(),
                body,
                headers: HashMap::from([("content-type".to_string(), content_type.clone())]),
                url: absolute,
                redirected: false,
            }));
        }
        return Err("Invalid data: URL (missing comma)".to_string());
    }

    if absolute == "about:blank" || absolute.is_empty() {
        return Ok(Some(ResolvedFetch {
            status: 200,
            status_text: "OK".to_string(),
            body: Vec::new(),
            url: absolute,
            headers: HashMap::from([("content-type".to_string(), "text/plain".to_string())]),
            redirected: false,
        }));
    }

    let parsed = RixUrl::parse(&absolute).map_err(|error| format!("Invalid fetch URL: {error}"))?;
    match parsed.scheme() {
        rix_core::url::RixScheme::Http | rix_core::url::RixScheme::Https => Ok(None),
        scheme => Err(format!("fetch: unsupported scheme '{scheme}'")),
    }
}

pub(super) enum FetchRequestBody {
    Bytes(Vec<u8>),
    Stream(rix_net::client::RequestBodyStream),
}

const REQUEST_BODY_QUEUE: usize = 4;

pub(super) struct RequestBodyUpload {
    reader: JsObject,
    read: Option<JsPromise>,
    pending: VecDeque<Vec<u8>>,
    chunks: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
    abort: Option<tokio::sync::oneshot::Sender<()>>,
    ended: bool,
    cancelled: bool,
    sent_bytes: usize,
    sent_chunks: usize,
    reads_issued: usize,
}

pub(super) enum UploadProgress {
    Running,
    Ended,
    Failed(String),
}

pub(super) fn is_readable_stream(value: &JsValue, context: &mut Context) -> JsResult<bool> {
    let Some(internals) = context
        .get_data::<super::streams::StreamsInternals>()
        .cloned()
    else {
        return Ok(false);
    };
    Ok(internals
        .is_readable_stream
        .call(&JsValue::undefined(), &[value.clone()], context)?
        .to_boolean())
}

pub(super) fn lock_request_body_stream(
    stream: &JsObject,
    context: &mut Context,
) -> JsResult<JsObject> {
    let reader = call_method(stream, "getReader", &[], context)?;
    reader.as_object().ok_or_else(|| {
        JsNativeError::typ()
            .with_message("fetch: request body stream has no reader")
            .into()
    })
}

fn request_body_chunk_bytes(
    value: &JsValue,
    context: &mut Context,
) -> JsResult<Option<Vec<u8>>> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    if let Ok(array) = JsTypedArray::from_object(object.clone()) {
        let offset = array.byte_offset(context)?;
        let length = array.byte_length(context)?;
        let buffer = array.buffer(context)?;
        let Some(buffer) = buffer
            .as_object()
            .and_then(|buffer| JsArrayBuffer::from_object(buffer).ok())
        else {
            return Ok(None);
        };
        return Ok(buffer_range(&buffer, offset, length));
    }
    if let Ok(view) = JsDataView::from_object(object.clone()) {
        let offset = usize::try_from(view.byte_offset(context)?).unwrap_or(usize::MAX);
        let length = usize::try_from(view.byte_length(context)?).unwrap_or(usize::MAX);
        let buffer = view.buffer(context)?;
        let Some(buffer) = buffer
            .as_object()
            .and_then(|buffer| JsArrayBuffer::from_object(buffer).ok())
        else {
            return Ok(None);
        };
        return Ok(buffer_range(&buffer, offset, length));
    }
    if let Ok(buffer) = JsArrayBuffer::from_object(object) {
        return Ok(buffer.data().map(|data| data.to_vec()));
    }
    Ok(None)
}

fn buffer_range(buffer: &JsArrayBuffer, offset: usize, length: usize) -> Option<Vec<u8>> {
    let data = buffer.data()?;
    let end = offset.checked_add(length)?;
    data.get(offset..end).map(<[u8]>::to_vec)
}

fn upload_reason_text(reason: &JsValue, context: &mut Context) -> String {
    if reason.is_undefined() {
        return "undefined".to_string();
    }
    reason
        .to_string(context)
        .map(|text| text.to_std_string_escaped())
        .unwrap_or_else(|_| "<unrenderable>".to_string())
}

fn mark_promise_handled(promise: &JsPromise, context: &mut Context) -> JsResult<()> {
    let handler = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(|_this, _args, _context| Ok(JsValue::undefined())),
    )
    .name(js_string!("request body promise outcome"))
    .build();
    promise.then(None, Some(handler), context)?;
    Ok(())
}

fn fail_upload_exchange(upload: &mut RequestBodyUpload) {
    upload.chunks = None;
    upload.ended = true;
    if let Some(abort) = upload.abort.take() {
        if abort.send(()).is_err() {
            tracing::debug!("request body abort raced the end of the exchange");
        }
    }
}

pub(super) fn pump_request_body(
    upload: &mut RequestBodyUpload,
    context: &mut Context,
) -> JsResult<UploadProgress> {
    for _ in 0..64 {
        if let Some(chunk) = upload.pending.pop_front() {
            let Some(chunks) = upload.chunks.as_ref() else {
                return Ok(UploadProgress::Ended);
            };
            match chunks.try_send(chunk) {
                Ok(()) => continue,
                Err(tokio::sync::mpsc::error::TrySendError::Full(chunk)) => {
                    upload.pending.push_front(chunk);
                    return Ok(UploadProgress::Running);
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    upload.chunks = None;
                    upload.ended = true;
                    return Ok(UploadProgress::Ended);
                }
            }
        }
        if upload.ended {
            return Ok(UploadProgress::Ended);
        }
        let Some(read) = upload.read.take() else {
            upload.reads_issued += 1;
            if upload.reads_issued <= 3 {
                tracing::debug!(
                    target: "rix_js::fetch::upload",
                    read = upload.reads_issued,
                    "request body pump asks the page stream for a chunk"
                );
            }
            let promise = call_method(&upload.reader, "read", &[], context)?;
            match promise
                .as_object()
                .and_then(|object| JsPromise::from_object(object.clone()).ok())
            {
                Some(promise) => {
                    mark_promise_handled(&promise, context)?;
                    upload.read = Some(promise);
                }
                None => {
                    fail_upload_exchange(upload);
                    return Ok(UploadProgress::Failed(
                        "fetch: request body reader returned no promise".to_string(),
                    ));
                }
            }
            continue;
        };
        match read.state() {
            PromiseState::Pending => {
                upload.read = Some(read);
                return Ok(UploadProgress::Running);
            }
            PromiseState::Rejected(reason) => {
                let message = upload_reason_text(&reason, context);
                fail_upload_exchange(upload);
                return Ok(UploadProgress::Failed(format!(
                    "fetch: request body stream errored: {message}"
                )));
            }
            PromiseState::Fulfilled(result) => {
                let Some(result) = result.as_object() else {
                    fail_upload_exchange(upload);
                    return Ok(UploadProgress::Failed(
                        "fetch: request body stream produced no read result".to_string(),
                    ));
                };
                let done = result.get(js_string!("done"), context)?.to_boolean();
                if done {
                    upload.chunks = None;
                    upload.abort = None;
                    upload.ended = true;
                    tracing::debug!(
                        target: "rix_js::fetch::upload",
                        chunks = upload.sent_chunks,
                        bytes = upload.sent_bytes,
                        "request body stream ended"
                    );
                    return Ok(UploadProgress::Ended);
                }
                let value = result.get(js_string!("value"), context)?;
                match request_body_chunk_bytes(&value, context)? {
                    Some(bytes) => {
                        upload.sent_bytes += bytes.len();
                        upload.sent_chunks += 1;
                        if upload.sent_chunks <= 8 || upload.sent_chunks % 64 == 0 {
                            tracing::debug!(
                                target: "rix_js::fetch::upload",
                                chunks = upload.sent_chunks,
                                bytes = upload.sent_bytes,
                                chunk = bytes.len(),
                                "request body chunk read from the page stream"
                            );
                        }
                        upload.pending.push_back(bytes);
                    }
                    None => {
                        fail_upload_exchange(upload);
                        return Ok(UploadProgress::Failed(
                            "fetch: this ReadableStream did not return bytes".to_string(),
                        ));
                    }
                }
            }
        }
    }
    Ok(UploadProgress::Running)
}

pub(super) fn abort_request_body(
    upload: &mut RequestBodyUpload,
    reason: JsValue,
    context: &mut Context,
) -> JsResult<()> {
    fail_upload_exchange(upload);
    if !upload.cancelled {
        upload.cancelled = true;
        let cancellation = call_method(&upload.reader, "cancel", &[reason], context)?;
        if let Some(promise) = cancellation
            .as_object()
            .and_then(|object| JsPromise::from_object(object).ok())
        {
            mark_promise_handled(&promise, context)?;
        }
    }
    Ok(())
}

pub(super) struct PendingFetch {
    pub(super) result: tokio::sync::mpsc::Receiver<FetchProgress>,
    pub(super) resolvers: ResolvingFunctions,
    pub(super) request_url: String,
    pub(super) method: String,
    pub(super) started_at: Instant,
    pub(super) abort_id: Option<u64>,
    pub(super) abort_signal: Option<JsObject>,
    pub(super) abort_registration: Option<JsValue>,
    pub(super) include_credentials: bool,
    pub(super) body: Option<StreamBody>,
    pub(super) upload: Option<RequestBodyUpload>,
}

pub(super) struct StreamBody {
    pub(super) controller: JsObject,
    pub(super) is_cancelled: JsObject,
    pub(super) pending: VecDeque<Vec<u8>>,
    pub(super) end_pending: bool,
    pub(super) error_pending: Option<StreamError>,
    pub(super) received: u64,
    pub(super) url: String,
    pub(super) status: u16,
}

pub(super) struct StreamError {
    pub(super) reason: JsValue,
    pub(super) trace: Option<String>,
}

pub(super) enum FetchProgress {
    Head(StreamHead),
    Chunk(Vec<u8>),
    End,
    Failed(String),
}

pub(super) struct StreamHead {
    pub(super) status: u16,
    pub(super) headers: HashMap<String, String>,
    pub(super) url: String,
    pub(super) redirected: bool,
    pub(super) set_cookies: Vec<String>,
}

pub(super) struct NetworkFetch {
    pub(super) url: RixUrl,
    pub(super) method: String,
    pub(super) headers: HashMap<String, String>,
    pub(super) body: FetchRequestBody,
    pub(super) result: tokio::sync::mpsc::Sender<FetchProgress>,
    pub(super) cancellation: Option<tokio::sync::watch::Receiver<bool>>,
    pub(super) cookie_store: Option<StorageEngine>,
    pub(super) head_timeout: Duration,
}

static SHARED_CLIENT: OnceLock<std::sync::RwLock<Option<rix_net::HttpClient>>> = OnceLock::new();

pub fn set_shared_network_client(client: rix_net::HttpClient) {
    let lock = SHARED_CLIENT.get_or_init(|| std::sync::RwLock::new(None));
    if let Ok(mut guard) = lock.write() {
        *guard = Some(client);
    }
}

pub fn get_shared_network_client() -> Option<rix_net::HttpClient> {
    SHARED_CLIENT
        .get()
        .and_then(|lock| lock.read().ok())
        .and_then(|guard| guard.clone())
}

pub(super) fn network_service() -> Result<&'static tokio::sync::mpsc::Sender<NetworkFetch>, String>
{
    static SERVICE: OnceLock<Result<tokio::sync::mpsc::Sender<NetworkFetch>, String>> =
        OnceLock::new();
    SERVICE
        .get_or_init(|| {
            let (sender, mut requests) = tokio::sync::mpsc::channel::<NetworkFetch>(256);
            std::thread::Builder::new()
                .name("rix-web-network".to_string())
                .spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            tracing::error!(%error, "web network runtime failed");
                            return;
                        }
                    };
                    runtime.block_on(async move {
                        let mut client = get_shared_network_client().unwrap_or_else(|| {
                            rix_net::HttpClient::new(None).expect("web network client failed")
                        });
                        let mut tasks = tokio::task::JoinSet::new();
                        loop {
                            tokio::select! {
                                request = requests.recv(), if tasks.len() < 64 => {
                                    let Some(request) = request else { break; };
                                    if let Some(shared) = get_shared_network_client() {
                                        client = shared;
                                    }
                                    let client = client.clone();
                                    tasks.spawn(async move {
                                        serve_fetch(client, request).await;
                                    });
                                }
                                _ = tasks.join_next(), if !tasks.is_empty() => {}
                            }
                        }
                    });
                })
                .map_err(|error| format!("fetch service: {error}"))?;
            Ok(sender)
        })
        .as_ref()
        .map_err(Clone::clone)
}

async fn serve_fetch(client: rix_net::HttpClient, request: NetworkFetch) {
    let NetworkFetch {
        url,
        method,
        headers,
        body,
        result,
        cancellation,
        cookie_store,
        head_timeout,
    } = request;
    let request_client = match cookie_store {
        Some(store) => client.with_storage(Some(store)),
        None => client,
    };

    let mut cancellation = cancellation;
    let requested_url = url.as_str().to_string();
    let requested_method = method.clone();
    let body_bytes = match &body {
        FetchRequestBody::Bytes(bytes) => bytes.len(),
        FetchRequestBody::Stream(_) => 0,
    };
    tracing::debug!(
        target: "rix_js::fetch",
        method = %method,
        streamed = matches!(body, FetchRequestBody::Stream(_)),
        body_bytes,
        head_timeout_ms = head_timeout.as_millis() as u64,
        "fetch exchange task started"
    );
    let is_get = method == "GET";
    let fetch_client = request_client.clone();
    let mut head_future = Box::pin(async move {
        match body {
            FetchRequestBody::Bytes(bytes) => {
                let head = tokio::time::timeout(
                    head_timeout,
                    fetch_client.request_stream(&url, &method, &headers, bytes),
                )
                .await;
                match head {
                    Ok(Ok(response)) => Ok(response),
                    Ok(Err(error)) => Err(format!("fetch failed: {error}")),
                    Err(_) => Err("fetch timed out".to_string()),
                }
            }
            FetchRequestBody::Stream(body) => fetch_client
                .request_stream_body(&url, &method, &headers, body)
                .await
                .map_err(|error| format!("fetch failed: {error}")),
        }
    });
    let fetch_started = std::time::Instant::now();
    let trace_enabled = std::env::var_os("RIX_WORKER_TRACE").is_some();
    if trace_enabled {
        eprintln!("[rix-fetch] start method={} url={}", requested_method, requested_url);
    }
    let head = if let Some(cancellation) = cancellation.as_mut() {
        tokio::select! {
            head = &mut head_future => head,
            _ = cancellation.changed() => {
                if *cancellation.borrow() {
                    return;
                }
                head_future.await
            }
        }
    } else {
        head_future.await
    };
    let mut stream = match head {
        Ok(response) => {
            if trace_enabled {
                eprintln!(
                    "[rix-fetch] head status={} elapsed={}ms url={}",
                    response.status,
                    fetch_started.elapsed().as_millis(),
                    requested_url
                );
            }
            response
        }
        Err(message) => {
            if trace_enabled {
                eprintln!(
                    "[rix-fetch] failed msg={} elapsed={}ms url={}",
                    message,
                    fetch_started.elapsed().as_millis(),
                    requested_url
                );
            }
            let _ = result.send(FetchProgress::Failed(message)).await;
            return;
        }
    };

    let head = FetchProgress::Head(StreamHead {
        status: stream.status,
        headers: stream.headers.clone(),
        url: stream.final_url.as_str().to_string(),
        redirected: stream.final_url.as_str() != requested_url,
        set_cookies: stream.set_cookies.clone(),
    });
    if result.send(head).await.is_err() {
        return;
    }

    let mut collected_body = Vec::new();
    let is_cacheable = is_get && stream.status == 200;

    loop {
        let chunk = if let Some(cancellation) = cancellation.as_mut() {
            tokio::select! {
                chunk = stream.body.next_chunk() => chunk,
                _ = cancellation.changed() => {
                    if *cancellation.borrow() {
                        return;
                    }
                    continue;
                }
            }
        } else {
            stream.body.next_chunk().await
        };
        let progress = match chunk {
            Some(Ok(bytes)) => {
                if is_cacheable && collected_body.len() + bytes.len() <= 2 * 1024 * 1024 {
                    collected_body.extend_from_slice(&bytes);
                }
                FetchProgress::Chunk(bytes.to_vec())
            }
            Some(Err(error)) => FetchProgress::Failed(format!("fetch body failed: {error}")),
            None => {
                if is_cacheable && !collected_body.is_empty() {
                    if let Some(cache) = request_client.http_cache() {
                        let response = rix_net::HttpResponse {
                            status: stream.status,
                            headers: stream.headers.clone(),
                            body: collected_body.clone(),
                            final_url: stream.final_url.clone(),
                            content_type: stream.content_type.clone(),
                            protocol: stream.protocol,
                            navigation_timing: stream.navigation_timing.clone(),
                            navigation_started: stream.navigation_started,
                            set_cookies: stream.set_cookies.clone(),
                        };
                        let _ = cache.put(&requested_url, &response);
                        if stream.final_url.as_str() != requested_url {
                            let _ = cache.put(stream.final_url.as_str(), &response);
                        }
                    }
                }
                FetchProgress::End
            }
        };
        let failed = !matches!(progress, FetchProgress::Chunk(_));
        if result.send(progress).await.is_err() || failed {
            return;
        }
    }
}

pub(super) enum BodyDelivery {
    Delivered,
    Full,
    Cancelled,
}

pub(super) fn build_fetch_body_stream(
    head: &StreamHead,
    abort_id: Option<u64>,
    context: &mut Context,
) -> JsResult<(JsObject, StreamBody)> {
    let internals = context
        .get_data::<super::streams::StreamsInternals>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("streams are unavailable"))?;
    let on_cancel = match abort_id {
        Some(id) => {
            let callback = FunctionObjectBuilder::new(
                context.realm(),
                NativeFunction::from_copy_closure_with_captures(
                    |_this, args, id, sub_context| {
                        let state = sub_context
                            .get_data::<Rc<RefCell<WebApiState>>>()
                            .cloned()
                            .ok_or_else(|| {
                                JsNativeError::typ()
                                    .with_message("fetch abort state is unavailable")
                            })?;
                        state
                            .borrow_mut()
                            .abort_fetch(*id, args.get_or_undefined(0).clone());
                        Ok(JsValue::undefined())
                    },
                    id,
                ),
            )
            .name(js_string!("fetch body cancel"))
            .build();
            callback.into()
        }
        None => JsValue::undefined(),
    };
    let pair = internals
        .create_push_stream
        .call(&JsValue::undefined(), &[on_cancel], context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("push stream helper returned no object")
        })?;
    let stream = pair.get(js_string!("stream"), context)?;
    let controller = pair
        .get(js_string!("controller"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("push stream helper returned no controller")
        })?;
    let is_cancelled = pair
        .get(js_string!("isCancelled"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("push stream helper returned no cancel probe")
        })?;
    let response = super::response::build_streaming_fetch_response(context, head, stream)?;
    let body = StreamBody {
        controller,
        is_cancelled,
        pending: VecDeque::new(),
        end_pending: false,
        error_pending: None,
        received: 0,
        url: head.url.clone(),
        status: head.status,
    };
    Ok((response, body))
}

const FETCH_BODY_QUEUE: usize = 4;

pub(super) enum BodyFlush {
    Drained,
    Blocked,
    Cancelled,
}

pub(super) fn flush_body_queue(
    body: &mut StreamBody,
    context: &mut Context,
) -> JsResult<BodyFlush> {
    while let Some(bytes) = body.pending.pop_front() {
        match deliver_body_chunk(body, &bytes, context)? {
            BodyDelivery::Delivered => {}
            BodyDelivery::Full => {
                body.pending.push_front(bytes);
                return Ok(BodyFlush::Blocked);
            }
            BodyDelivery::Cancelled => return Ok(BodyFlush::Cancelled),
        }
    }
    Ok(BodyFlush::Drained)
}

fn call_method(
    receiver: &JsObject,
    name: &str,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let method = receiver.get(JsString::from(name), context)?;
    let method = method
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message(format!("{name} is not a function")))?;
    method.call(&JsValue::from(receiver.clone()), args, context)
}

pub(super) fn deliver_body_chunk(
    body: &mut StreamBody,
    bytes: &[u8],
    context: &mut Context,
) -> JsResult<BodyDelivery> {
    let cancelled = body
        .is_cancelled
        .call(&JsValue::undefined(), &[], context)?
        .to_boolean();
    if cancelled {
        return Ok(BodyDelivery::Cancelled);
    }
    let desired = body.controller.get(js_string!("desiredSize"), context)?;
    if desired.is_null() {
        return Ok(BodyDelivery::Cancelled);
    }
    if desired.as_number().is_none_or(|size| size <= 0.0) {
        return Ok(BodyDelivery::Full);
    }
    let length = bytes.len() as u64;
    let chunk = JsUint8Array::from_iter(bytes.iter().copied(), context)?;
    call_method(&body.controller, "enqueue", &[chunk.into()], context)?;
    body.received += length;
    Ok(BodyDelivery::Delivered)
}

pub(super) fn close_body_stream(body: &StreamBody, context: &mut Context) -> JsResult<()> {
    call_method(&body.controller, "close", &[], context)?;
    Ok(())
}

pub(super) fn error_body_stream(
    body: &StreamBody,
    reason: JsValue,
    context: &mut Context,
) -> JsResult<()> {
    call_method(&body.controller, "error", &[reason], context)?;
    Ok(())
}

pub(super) fn default_abort_reason(context: &mut Context) -> JsResult<JsValue> {
    let constructor = context
        .global_object()
        .get(js_string!("DOMException"), context)?;
    let constructor = constructor
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("DOMException is not a constructor"))?;
    constructor.call(
        &JsValue::undefined(),
        &[
            js_string!("This operation was aborted").into(),
            js_string!("AbortError").into(),
        ],
        context,
    )
}

pub(super) fn settle_fetch(
    resolvers: ResolvingFunctions,
    result: Result<ResolvedFetch, String>,
    context: &mut Context,
) -> JsResult<()> {
    match result {
        Ok(response) => match super::response::build_fetch_response(context, &response) {
            Ok(response) => {
                resolvers
                    .resolve
                    .call(&JsValue::undefined(), &[response.into()], context)?;
            }
            Err(error) => {
                let reason = match error.into_opaque(context) {
                    Ok(reason) => reason,
                    Err(engine_error) => JsNativeError::error()
                        .with_message(engine_error.to_string())
                        .into_opaque(context)
                        .into(),
                };
                resolvers
                    .reject
                    .call(&JsValue::undefined(), &[reason], context)?;
            }
        },
        Err(message) => {
            let error = JsNativeError::typ()
                .with_message(message)
                .into_opaque(context);
            resolvers
                .reject
                .call(&JsValue::undefined(), &[error.into()], context)?;
        }
    }
    Ok(())
}

fn extract_headers_from_js_object(
    obj: &JsObject,
    headers: &mut HashMap<String, String>,
    context: &mut Context,
) -> JsResult<()> {
    let is_headers = obj
        .get(boa_engine::symbol::JsSymbol::to_string_tag(), context)
        .ok()
        .and_then(|tag| tag.as_string().map(|s| s.to_std_string_escaped()))
        .as_deref()
        == Some("Headers");

    if is_headers {
        if let Ok(entries_method) = obj.get(js_string!("entries"), context) {
            if let Some(entries_fn) = entries_method.as_object() {
                if let Ok(entries_iter) = entries_fn.call(&obj.clone().into(), &[], context) {
                    if let Some(iter_obj) = entries_iter.as_object() {
                        if let Ok(next_method) = iter_obj.get(js_string!("next"), context) {
                            if let Some(next_fn) = next_method.as_object() {
                                loop {
                                    let result =
                                        next_fn.call(&iter_obj.clone().into(), &[], context)?;
                                    if let Some(res_obj) = result.as_object() {
                                        let done =
                                            res_obj.get(js_string!("done"), context)?.to_boolean();
                                        if done {
                                            break;
                                        }
                                        let value = res_obj.get(js_string!("value"), context)?;
                                        if let Some(pair_obj) = value.as_object() {
                                            let name = pair_obj
                                                .get(0, context)?
                                                .to_string(context)?
                                                .to_std_string_escaped();
                                            let val = pair_obj
                                                .get(1, context)?
                                                .to_string(context)?
                                                .to_std_string_escaped();
                                            headers.insert(name, val);
                                        }
                                    } else {
                                        break;
                                    }
                                }
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
    }

    if obj.is_array() {
        if let Ok(len_val) = obj.get(js_string!("length"), context) {
            let len = len_val.to_u32(context).unwrap_or(0);
            for i in 0..len {
                if let Ok(item) = obj.get(i, context) {
                    if let Some(pair_obj) = item.as_object() {
                        let name = pair_obj
                            .get(0, context)?
                            .to_string(context)?
                            .to_std_string_escaped();
                        let value = pair_obj
                            .get(1, context)?
                            .to_string(context)?
                            .to_std_string_escaped();
                        headers.insert(name, value);
                    }
                }
            }
            return Ok(());
        }
    }

    for key in obj.own_property_keys(context)? {
        if let boa_engine::property::PropertyKey::String(name) = &key {
            let value = obj
                .get(key.clone(), context)?
                .to_string(context)?
                .to_std_string_escaped();
            headers.insert(name.to_std_string_escaped(), value);
        }
    }
    Ok(())
}

pub(super) fn fetch_fn(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let _ = this;
    let arg0 = args.get_or_undefined(0);
    let mut url_str = String::new();
    let mut method = "GET".to_string();
    let mut headers = HashMap::new();
    let mut body = Vec::new();
    let mut body_reader: Option<JsObject> = None;
    let mut signal = None;
    let mut credentials = "same-origin".to_string();
    let mut mode = "cors".to_string();

    if let Some(req_obj) = arg0.as_object() {
        let is_request = req_obj
            .get(boa_engine::symbol::JsSymbol::to_string_tag(), context)
            .ok()
            .and_then(|tag| tag.as_string().map(|s| s.to_std_string_escaped()))
            .as_deref()
            == Some("Request");

        if is_request {
            if let Ok(url_val) = req_obj.get(js_string!("url"), context) {
                if !url_val.is_undefined() {
                    url_str = url_val.to_string(context)?.to_std_string_escaped();
                }
            }
            if let Ok(method_val) = req_obj.get(js_string!("method"), context) {
                if !method_val.is_undefined() {
                    method = method_val
                        .to_string(context)?
                        .to_std_string_escaped()
                        .to_ascii_uppercase();
                }
            }
            if let Ok(sig_val) = req_obj.get(js_string!("signal"), context) {
                if let Some(sig_obj) = sig_val.as_object() {
                    read_abort_state(&sig_obj, context)?;
                    signal = Some(sig_obj);
                }
            }
            if let Ok(cred_val) = req_obj.get(js_string!("credentials"), context) {
                if !cred_val.is_undefined() {
                    credentials = cred_val.to_string(context)?.to_std_string_escaped();
                }
            }
            if let Ok(mode_val) = req_obj.get(js_string!("mode"), context) {
                if !mode_val.is_undefined() {
                    let m = mode_val
                        .to_string(context)?
                        .to_std_string_escaped()
                        .to_lowercase();
                    if m == "cors" || m == "no-cors" || m == "same-origin" || m == "navigate" {
                        mode = m;
                    }
                }
            }
            if let Ok(h_val) = req_obj.get(js_string!("headers"), context) {
                if let Some(h_obj) = h_val.as_object() {
                    extract_headers_from_js_object(&h_obj, &mut headers, context)?;
                }
            }
            if let Ok(used_val) = req_obj.get(js_string!("bodyUsed"), context) {
                let options_has_body = args
                    .get(1)
                    .and_then(JsValue::as_object)
                    .and_then(|o| o.get(js_string!("body"), context).ok())
                    .map_or(false, |b| !b.is_null_or_undefined());
                if used_val.to_boolean() && !options_has_body {
                    return rejected_fetch(
                        JsNativeError::typ()
                            .with_message(
                                "Cannot construct a Request with a Request object that has already been used",
                            )
                            .into(),
                        context,
                    );
                }
            }
            let mut request_carries_bytes = false;
            if let Ok(bytes_val) = req_obj.get(js_string!("__rix_request_body_bytes"), context) {
                if let Some(bytes_obj) = bytes_val.as_object() {
                    if let Ok(typed_array) = JsUint8Array::from_object(bytes_obj) {
                        if let Ok(vec) = typed_array.to_vec(context) {
                            body = vec;
                            request_carries_bytes = true;
                        }
                    }
                }
            }
            if !request_carries_bytes {
                if let Ok(stream_val) =
                    req_obj.get(js_string!("__rix_request_body_stream"), context)
                {
                    if let Some(stream) = stream_val.as_object() {
                        match lock_request_body_stream(&stream, context) {
                            Ok(reader) => body_reader = Some(reader),
                            Err(error) => return rejected_fetch(error, context),
                        }
                    }
                }
            }
            if let Ok(mark_method) = req_obj.get(js_string!("__rix_mark_used"), context) {
                if let Some(mark_fn) = mark_method.as_object() {
                    let _ = mark_fn.call(&req_obj.clone().into(), &[], context);
                }
            }
        } else {
            url_str = arg0.to_string(context)?.to_std_string_escaped();
        }
    } else {
        url_str = arg0.to_string(context)?.to_std_string_escaped();
    }

    if let Some(options) = args.get(1).and_then(JsValue::as_object) {
        let candidate = options.get(js_string!("method"), context)?;
        if !candidate.is_undefined() {
            method = candidate
                .to_string(context)?
                .to_std_string_escaped()
                .to_ascii_uppercase();
        }
        let supplied = options.get(js_string!("headers"), context)?;
        if let Some(supplied) = supplied.as_object() {
            extract_headers_from_js_object(&supplied, &mut headers, context)?;
        }
        let duplex = options.get(js_string!("duplex"), context)?;
        let duplex_half = if duplex.is_undefined() {
            false
        } else {
            let value = duplex.to_string(context)?.to_std_string_escaped();
            if value != "half" {
                return rejected_fetch(
                    JsNativeError::typ()
                        .with_message(format!(
                            "Failed to read the 'duplex' property from 'RequestInit': The provided value '{value}' is not a valid enum value of type RequestDuplex."
                        ))
                        .into(),
                    context,
                );
            }
            true
        };
        let value = options.get(js_string!("body"), context)?;
        if !value.is_undefined() && !value.is_null() {
            body.clear();
            body_reader = None;
            if is_readable_stream(&value, context)? {
                if !duplex_half {
                    return rejected_fetch(
                        JsNativeError::typ()
                            .with_message(
                                "The `duplex` member must be specified for a request with a streaming body",
                            )
                            .into(),
                        context,
                    );
                }
                let Some(stream) = value.as_object() else {
                    return rejected_fetch(
                        JsNativeError::typ()
                            .with_message("The provided value is not a ReadableStream")
                            .into(),
                        context,
                    );
                };
                match lock_request_body_stream(&stream, context) {
                    Ok(reader) => body_reader = Some(reader),
                    Err(error) => return rejected_fetch(error, context),
                }
            } else if let Some(obj) = value.as_object() {
                if let Ok(typed_array) = JsUint8Array::from_object(obj.clone()) {
                    body = typed_array.to_vec(context)?;
                } else {
                    body = value
                        .to_string(context)?
                        .to_std_string_escaped()
                        .into_bytes();
                }
            } else {
                body = value
                    .to_string(context)?
                    .to_std_string_escaped()
                    .into_bytes();
            }
        }
        let cred = options.get(js_string!("credentials"), context)?;
        if !cred.is_undefined() {
            credentials = cred.to_string(context)?.to_std_string_escaped();
        }
        let mode_val = options.get(js_string!("mode"), context)?;
        if !mode_val.is_undefined() {
            let m = mode_val
                .to_string(context)?
                .to_std_string_escaped()
                .to_lowercase();
            if m == "cors" || m == "no-cors" || m == "same-origin" || m == "navigate" {
                mode = m;
            }
        }
        let candidate = options.get(js_string!("signal"), context)?;
        if !candidate.is_null_or_undefined() {
            let candidate = candidate.as_object().ok_or_else(|| {
                JsNativeError::typ().with_message("fetch signal must be an AbortSignal")
            })?;
            read_abort_state(&candidate, context)?;
            signal = Some(candidate);
        }
    }
    let base = context
        .get_data::<Rc<RefCell<WebApiState>>>()
        .and_then(|s| s.borrow().current_url.clone());

    let absolute = match &base {
        Some(b) => b
            .join(&url_str)
            .map(|url| url.as_str().to_string())
            .unwrap_or(url_str),
        None => url_str,
    };
    let (promise, resolvers) = JsPromise::new_pending(context);
    let started_at = Instant::now();

    if let Some(signal) = &signal {
        let (aborted, reason) = read_abort_state(signal, context)?;
        if aborted {
            record_fetch_load(
                context,
                ResourceLoadRecord::new(
                    ResourceLoadKind::Fetch,
                    &method,
                    &absolute,
                    None,
                    None,
                    started_at.elapsed(),
                    None,
                    Some("aborted"),
                ),
            );
            reject_fetch(resolvers, reason, context)?;
            return Ok(promise.into());
        }
    }

    if absolute.starts_with("blob:") {
        let stored = context
            .get_data::<Rc<RefCell<crate::web_api::blob::BlobUrlStore>>>()
            .and_then(|store| store.borrow().resolve_blob_url(&absolute));
        match stored {
            Some(stored_blob) => {
                let mime = if stored_blob.mime_type.is_empty() {
                    "application/octet-stream".to_string()
                } else {
                    stored_blob.mime_type.clone()
                };
                let body_len = stored_blob.bytes.len();
                let response = ResolvedFetch {
                    status: 200,
                    status_text: "OK".to_string(),
                    body: stored_blob.bytes,
                    url: absolute.clone(),
                    headers: HashMap::from([
                        ("content-type".to_string(), mime),
                        ("content-length".to_string(), body_len.to_string()),
                    ]),
                    redirected: false,
                };
                record_fetch_load(
                    context,
                    ResourceLoadRecord::new(
                        ResourceLoadKind::Fetch,
                        &method,
                        &absolute,
                        Some(&response.url),
                        Some(response.status),
                        started_at.elapsed(),
                        Some(body_len),
                        None,
                    ),
                );
                settle_fetch(resolvers, Ok(response), context)?;
                return Ok(promise.into());
            }
            None => {
                let error = "Failed to fetch blob URL: resource not found or revoked";
                record_fetch_load(
                    context,
                    ResourceLoadRecord::new(
                        ResourceLoadKind::Fetch,
                        &method,
                        &absolute,
                        None,
                        None,
                        started_at.elapsed(),
                        None,
                        Some(error),
                    ),
                );
                settle_fetch(resolvers, Err(error.into()), context)?;
                return Ok(promise.into());
            }
        }
    }

    match resolve_local_fetch(&absolute) {
        Ok(Some(response)) => {
            let error =
                (response.status >= 400).then(|| format!("HTTP status {}", response.status));
            record_fetch_load(
                context,
                ResourceLoadRecord::new(
                    ResourceLoadKind::Fetch,
                    &method,
                    &absolute,
                    Some(&response.url),
                    Some(response.status),
                    started_at.elapsed(),
                    Some(response.body.len()),
                    error.as_deref(),
                ),
            );
            settle_fetch(resolvers, Ok(response), context)?;
            return Ok(promise.into());
        }
        Ok(None) => {}
        Err(error) => {
            record_fetch_load(
                context,
                ResourceLoadRecord::new(
                    ResourceLoadKind::Fetch,
                    &method,
                    &absolute,
                    None,
                    None,
                    started_at.elapsed(),
                    None,
                    Some(&error),
                ),
            );
            settle_fetch(resolvers, Err(error), context)?;
            return Ok(promise.into());
        }
    }

    let Some(state) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() else {
        let error = "fetch requires a browser event loop".to_string();
        record_fetch_load(
            context,
            ResourceLoadRecord::new(
                ResourceLoadKind::Fetch,
                &method,
                &absolute,
                None,
                None,
                started_at.elapsed(),
                None,
                Some(&error),
            ),
        );
        settle_fetch(resolvers, Err(error), context)?;
        return Ok(promise.into());
    };

    let (cancel_sender, cancel_receiver) = tokio::sync::watch::channel(false);
    let abort_id = state
        .borrow_mut()
        .register_fetch_cancellation(cancel_sender);
    let (abort_signal, abort_registration) = if let Some(signal) = &signal {
        let algorithm = FunctionObjectBuilder::new(
            context.realm(),
            NativeFunction::from_copy_closure_with_captures(
                |_this, args, id, sub_context| {
                    let state = sub_context
                        .get_data::<Rc<RefCell<WebApiState>>>()
                        .cloned()
                        .ok_or_else(|| {
                            JsNativeError::typ().with_message("fetch abort state is unavailable")
                        })?;
                    state
                        .borrow_mut()
                        .abort_fetch(*id, args.get_or_undefined(0).clone());
                    Ok(JsValue::undefined())
                },
                abort_id,
            ),
        )
        .name(js_string!("fetch abort algorithm"))
        .build();
        let registration = match add_abort_algorithm(signal, &algorithm, context) {
            Ok(registration) => registration,
            Err(error) => {
                state.borrow_mut().finish_fetch_cancellation(abort_id);
                return Err(error);
            }
        };

        let abort_reason = state.borrow().fetch_abort_reason(abort_id);
        if let Some(reason) = abort_reason {
            state.borrow_mut().finish_fetch_cancellation(abort_id);
            record_fetch_load(
                context,
                ResourceLoadRecord::new(
                    ResourceLoadKind::Fetch,
                    &method,
                    &absolute,
                    None,
                    None,
                    started_at.elapsed(),
                    None,
                    Some("aborted"),
                ),
            );
            reject_fetch(resolvers, reason, context)?;
            return Ok(promise.into());
        }

        (Some(signal.clone()), Some(registration))
    } else {
        (None, None)
    };

    let (sender, receiver) = tokio::sync::mpsc::channel(FETCH_BODY_QUEUE);
    let request_url = absolute.clone();
    let trace_method = method.clone();

    let (request_body, upload) = match body_reader {
        Some(reader) => {
            let (chunks, chunk_receiver) = tokio::sync::mpsc::channel(REQUEST_BODY_QUEUE);
            let (abort, abort_receiver) = tokio::sync::oneshot::channel();
            (
                FetchRequestBody::Stream(rix_net::client::RequestBodyStream::new(
                    chunk_receiver,
                    abort_receiver,
                )),
                Some(RequestBodyUpload {
                    reader,
                    read: None,
                    pending: VecDeque::new(),
                    chunks: Some(chunks),
                    abort: Some(abort),
                    ended: false,
                    cancelled: false,
                    sent_bytes: 0,
                    sent_chunks: 0,
                    reads_issued: 0,
                }),
            )
        }
        None => (FetchRequestBody::Bytes(body), None),
    };

    let target_parsed = RixUrl::parse(&absolute).ok();
    let is_same_origin = match (&base, &target_parsed) {
        (Some(b), Some(t)) => rix_net::client::same_origin(b, t),
        _ => true,
    };
    let include_credentials = match credentials.as_str() {
        "omit" => false,
        "include" => true,
        "same-origin" => is_same_origin,
        _ => is_same_origin,
    };
    let cookie_store = if include_credentials {
        state.borrow().cookie_store.clone()
    } else {
        None
    };

    let site = target_parsed
        .as_ref()
        .map(|target| rix_net::client::compute_sec_fetch_site(base.as_ref(), target))
        .unwrap_or("none");
    headers.insert("sec-fetch-site".to_string(), site.to_string());
    headers.insert("sec-fetch-mode".to_string(), mode.clone());
    headers.insert("sec-fetch-dest".to_string(), "empty".to_string());
    headers.remove("cookie");
    headers.remove("cookie2");
    headers.remove("host");
    headers.remove("content-length");

    let request = target_parsed
        .ok_or_else(|| format!("fetch URL: invalid URL '{absolute}'"))
        .and_then(|url| {
            network_service()?
                .try_send(NetworkFetch {
                    url,
                    method,
                    headers,
                    body: request_body,
                    result: sender,
                    cancellation: Some(cancel_receiver),
                    cookie_store,
                    head_timeout: state.borrow().fetch_head_timeout,
                })
                .map_err(|error| format!("fetch queue: {error}"))
        });
    match request {
        Ok(()) => {
            state.borrow_mut().pending_fetches.push(PendingFetch {
                result: receiver,
                resolvers,
                request_url,
                method: trace_method,
                started_at,
                abort_id: Some(abort_id),
                abort_signal,
                abort_registration,
                include_credentials,
                body: None,
                upload,
            });
        }
        Err(error) => {
            if let Some(mut upload) = upload {
                if let Err(cancel_error) =
                    abort_request_body(&mut upload, JsValue::undefined(), context)
                {
                    tracing::debug!(
                        %cancel_error,
                        "request body cancel failed after the fetch enqueue error"
                    );
                }
            }
            if let (Some(signal), Some(registration)) = (&abort_signal, &abort_registration) {
                remove_abort_algorithm(signal, registration, context)?;
            }
            state.borrow_mut().finish_fetch_cancellation(abort_id);
            record_fetch_load(
                context,
                ResourceLoadRecord::new(
                    ResourceLoadKind::Fetch,
                    &trace_method,
                    &absolute,
                    None,
                    None,
                    started_at.elapsed(),
                    None,
                    Some(&error),
                ),
            );
            settle_fetch(resolvers, Err(error), context)?;
        }
    }
    Ok(promise.into())
}

pub(super) fn reject_fetch(
    resolvers: ResolvingFunctions,
    reason: JsValue,
    context: &mut Context,
) -> JsResult<()> {
    resolvers
        .reject
        .call(&JsValue::undefined(), &[reason], context)?;
    Ok(())
}

fn rejected_fetch(reason: JsError, context: &mut Context) -> JsResult<JsValue> {
    let (promise, resolvers) = JsPromise::new_pending(context);
    let reason = match reason.into_opaque(context) {
        Ok(reason) => reason,
        Err(error) => JsNativeError::error()
            .with_message(error.to_string())
            .into_opaque(context)
            .into(),
    };
    reject_fetch(resolvers, reason, context)?;
    Ok(promise.into())
}

fn record_fetch_load(context: &mut Context, record: ResourceLoadRecord) {
    if let Some(state) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        state.borrow_mut().record_resource_load(record);
    }
}
