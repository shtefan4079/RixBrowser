
use crate::decompress::{
    MAX_DECOMPRESSED_BYTES, StreamingDecoder, decompress_body_for_header_limited,
};
use crate::dns_tls::{DnsResolver, NegotiatedProtocol, TlsManager};
use crate::error::{NetError, Result};
use crate::timing::{CompletedTiming, FetchClock, NavigationTiming};
use bytes::{Buf, Bytes};
use http_body_util::{BodyExt, Full};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use hyper::header::{
    ACCEPT, ACCEPT_ENCODING, COOKIE, HOST, HeaderMap, HeaderValue, SET_COOKIE, USER_AGENT,
};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use rix_core::url::{RixScheme, RixUrl};
use rix_storage::StorageEngine;
use rix_storage::models::{Cookie, SameSitePolicy};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

const H3_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HAPPY_EYEBALLS_DELAY: Duration = Duration::from_millis(25);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub type H3Sender = h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub final_url: RixUrl,
    pub content_type: String,
    pub protocol: NegotiatedProtocol,
    pub navigation_timing: NavigationTiming,
    pub navigation_started: Instant,
    pub set_cookies: Vec<String>,
}

impl HttpResponse {
    pub fn navigation_elapsed_ms(&self) -> f64 {
        self.navigation_started.elapsed().as_secs_f64() * 1000.0
    }

    pub fn text(&self) -> std::result::Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.body)
    }

    pub fn is_not_modified(&self) -> bool {
        self.status == 304
    }
}

#[derive(Debug, Clone, Default)]
pub struct ConditionalRequest {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl ConditionalRequest {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.etag.is_none() && self.last_modified.is_none()
    }
}

pub struct StreamingResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub final_url: RixUrl,
    pub content_type: String,
    pub protocol: NegotiatedProtocol,
    pub navigation_timing: NavigationTiming,
    pub navigation_started: Instant,
    pub set_cookies: Vec<String>,
    pub body: BodyStream,
}

impl StreamingResponse {
    pub fn from_cached_entry(entry: &crate::cache::HttpCacheEntry, url: RixUrl) -> Self {
        let final_url = RixUrl::parse(&entry.meta.final_url).unwrap_or_else(|_| url.clone());
        Self {
            status: entry.meta.status,
            headers: entry.meta.headers.clone(),
            final_url,
            content_type: entry.meta.content_type.clone(),
            protocol: entry.meta.protocol,
            navigation_timing: NavigationTiming::default(),
            navigation_started: Instant::now(),
            set_cookies: Vec::new(),
            body: BodyStream::from_bytes(entry.body.clone()),
        }
    }
}

pub struct BodyStream {
    body: Option<hyper::body::Incoming>,
    decoder: StreamingDecoder,
    decoded: Vec<u8>,
    release: Option<(ConnectionPool, String, PooledSender)>,
    cached: Option<Bytes>,
    done: bool,
}

impl BodyStream {
    pub fn from_bytes(bytes: Bytes) -> Self {
        Self {
            body: None,
            decoder: StreamingDecoder::for_header("", None).expect("identity decoder"),
            decoded: Vec::new(),
            release: None,
            cached: Some(bytes),
            done: false,
        }
    }

    pub async fn next_chunk(&mut self) -> Option<Result<Bytes>> {
        if self.done {
            return None;
        }
        if let Some(bytes) = self.cached.take() {
            self.done = true;
            return Some(Ok(bytes));
        }
        let Some(body) = self.body.as_mut() else {
            self.done = true;
            return None;
        };
        loop {
            let frame = match body.frame().await {
                Some(Ok(frame)) => frame,
                Some(Err(error)) => {
                    self.done = true;
                    self.release_connection().await;
                    return Some(Err(NetError::Http(format!("response body: {error}"))));
                }
                None => {
                    if let Err(error) = self.decoder.finish(&mut self.decoded) {
                        self.done = true;
                        self.release_connection().await;
                        return Some(Err(error));
                    }
                    self.done = true;
                    self.release_connection().await;
                    if self.decoded.is_empty() {
                        return None;
                    }
                    let chunk = Bytes::from(std::mem::take(&mut self.decoded));
                    return Some(Ok(chunk));
                }
            };
            let Some(data) = frame.data_ref() else {
                continue;
            };
            if data.is_empty() {
                continue;
            }
            if let Err(error) = self.decoder.push(data, &mut self.decoded) {
                self.done = true;
                self.release_connection().await;
                return Some(Err(error));
            }
            if !self.decoded.is_empty() {
                let chunk = Bytes::from(std::mem::take(&mut self.decoded));
                return Some(Ok(chunk));
            }
        }
    }

    async fn release_connection(&mut self) {
        if let Some((pool, key, sender)) = self.release.take() {
            return_sender(&pool, &key, sender).await;
        }
    }
}

pub struct RequestBodyStream {
    chunks: tokio::sync::mpsc::Receiver<Vec<u8>>,
    abort: Option<tokio::sync::oneshot::Receiver<()>>,
}

impl RequestBodyStream {
    pub fn new(
        chunks: tokio::sync::mpsc::Receiver<Vec<u8>>,
        abort: tokio::sync::oneshot::Receiver<()>,
    ) -> Self {
        Self {
            chunks,
            abort: Some(abort),
        }
    }
}

impl hyper::body::Body for RequestBodyStream {
    type Data = Bytes;
    type Error = NetError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<Option<std::result::Result<hyper::body::Frame<Bytes>, NetError>>> {
        let this = self.get_mut();
        if let Some(abort) = this.abort.as_mut() {
            match Pin::new(abort).poll(cx) {
                Poll::Ready(Ok(())) => {
                    this.abort = None;
                    return Poll::Ready(Some(Err(NetError::Http(
                        "request body was aborted".to_string(),
                    ))));
                }
                Poll::Ready(Err(_)) => this.abort = None,
                Poll::Pending => {}
            }
        }
        match this.chunks.poll_recv(cx) {
            Poll::Ready(Some(bytes)) => Poll::Ready(Some(Ok(hyper::body::Frame::data(
                Bytes::from(bytes),
            )))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

struct StreamedExchange {
    parts: hyper::http::response::Parts,
    body: hyper::body::Incoming,
    protocol: NegotiatedProtocol,
    pool_key: String,
    sender: Option<PooledSender>,
}

enum PooledSender {
    Http1(hyper::client::conn::http1::SendRequest<Full<Bytes>>),
    Http2(hyper::client::conn::http2::SendRequest<Full<Bytes>>),
    Http3(H3Sender),
}

#[derive(Default, Clone)]
pub struct ConnectionPool {
    pool: Arc<Mutex<HashMap<String, Vec<PooledSender>>>>,
}

impl ConnectionPool {
    pub fn new() -> Self {
        Self::default()
    }

    async fn acquire_tcp(&self, key: &str) -> Option<PooledSender> {
        let mut guard = self.pool.lock().await;
        let senders = guard.get_mut(key)?;
        let mut skipped_h3 = Vec::new();

        while let Some(sender) = senders.pop() {
            match sender {
                PooledSender::Http2(h2) => {
                    if !h2.is_closed() {
                        senders.push(PooledSender::Http2(h2.clone()));
                        senders.extend(skipped_h3);
                        return Some(PooledSender::Http2(h2));
                    }
                }
                PooledSender::Http1(h1) => {
                    if h1.is_ready() {
                        senders.extend(skipped_h3);
                        return Some(PooledSender::Http1(h1));
                    }
                }
                PooledSender::Http3(h3) => {
                    skipped_h3.push(PooledSender::Http3(h3));
                }
            }
        }
        senders.extend(skipped_h3);
        None
    }

    async fn acquire_h3(&self, key: &str) -> Option<H3Sender> {
        let mut guard = self.pool.lock().await;
        let senders = guard.get_mut(key)?;
        let pos = senders
            .iter()
            .rposition(|s| matches!(s, PooledSender::Http3(_)))?;
        let sender = senders.remove(pos);
        match sender {
            PooledSender::Http3(h3) => {
                senders.push(PooledSender::Http3(h3.clone()));
                Some(h3)
            }
            _ => None,
        }
    }

    async fn release(&self, key: String, sender: PooledSender) {
        let mut guard = self.pool.lock().await;
        guard.entry(key).or_default().push(sender);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H3OriginState {
    Advertised(u16),
    Supported(u16),
    Unsupported,
}

#[derive(Clone)]
pub struct HttpClient {
    dns: DnsResolver,
    tls: TlsManager,
    storage: Option<StorageEngine>,
    pool: ConnectionPool,
    h3_state: Arc<Mutex<HashMap<String, H3OriginState>>>,
    quinn_endpoint: Option<quinn::Endpoint>,
    user_agent: String,
    accept_language: String,
    max_redirects: usize,
    request_timeout: Duration,
    max_response_bytes: usize,
    request_method: Method,
    request_headers: HeaderMap,
    request_body: Bytes,
    http_cache: Option<Arc<crate::cache::HttpCache>>,
}

impl HttpClient {
    pub fn new(storage: Option<StorageEngine>) -> Result<Self> {
        let dns = DnsResolver::new()?;
        let tls = TlsManager::new()?;
        let quinn_endpoint = create_quic_endpoint(&tls);
        let profile = rix_core::BrowserProfile::system_default();
        let http_cache = Some(Arc::new(crate::cache::HttpCache::default_disk_cache()));

        Ok(Self {
            dns,
            tls,
            storage,
            pool: ConnectionPool::new(),
            h3_state: Arc::new(Mutex::new(HashMap::new())),
            quinn_endpoint,
            user_agent: profile.user_agent().to_owned(),
            accept_language: profile.accept_language(),
            max_redirects: 10,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_response_bytes: MAX_DECOMPRESSED_BYTES,
            request_method: Method::GET,
            request_headers: HeaderMap::new(),
            request_body: Bytes::new(),
            http_cache,
        })
    }

    pub fn pool(&self) -> &ConnectionPool {
        &self.pool
    }

    pub fn h3_available(&self) -> bool {
        self.quinn_endpoint.is_some()
    }

    pub fn with_user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = ua.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn with_max_response_bytes(mut self, limit: usize) -> Self {
        self.max_response_bytes = limit;
        self
    }

    pub fn storage(&self) -> Option<&StorageEngine> {
        self.storage.as_ref()
    }

    pub fn set_storage(&mut self, storage: Option<StorageEngine>) {
        self.storage = storage;
    }

    pub fn with_storage(mut self, storage: Option<StorageEngine>) -> Self {
        self.storage = storage;
        self
    }

    pub fn http_cache(&self) -> Option<&Arc<crate::cache::HttpCache>> {
        self.http_cache.as_ref()
    }

    pub fn with_http_cache(mut self, cache: Option<Arc<crate::cache::HttpCache>>) -> Self {
        self.http_cache = cache;
        self
    }

    pub fn set_http_cache(&mut self, cache: Option<Arc<crate::cache::HttpCache>>) {
        self.http_cache = cache;
    }

    pub fn cache_response(&self, requested_url: &str, response: &HttpResponse) {
        if let Some(cache) = &self.http_cache {
            if response.status == 200 {
                let _ = cache.put(requested_url, response);
                if response.final_url.as_str() != requested_url {
                    let _ = cache.put(response.final_url.as_str(), response);
                }
            }
        }
    }

    pub async fn dns_prefetch(&self, host: &str) {
        let _ = self.dns.resolve(host).await;
    }

    pub async fn preconnect(&self, url: &RixUrl) {
        let is_https = url.scheme() == RixScheme::Https;
        let host = match url.host_str() {
            Some(h) => h.to_string(),
            None => return,
        };
        let default_port = if is_https { 443 } else { 80 };
        let port = url.port().unwrap_or(default_port);
        let pool_key = format!("{}:{host}:{port}", if is_https { "https" } else { "http" });

        if let Some(existing) = self.pool.acquire_tcp(&pool_key).await {
            self.pool.release(pool_key, existing).await;
            return;
        }

        let addrs = match self.dns.resolve_socket_addrs(&host, port).await {
            Ok(a) if !a.is_empty() => a,
            _ => return,
        };

        let (tcp_stream, _) = match connect_happy_eyeballs(&addrs, HAPPY_EYEBALLS_DELAY, TCP_CONNECT_TIMEOUT).await {
            Ok(stream) => stream,
            Err(_) => return,
        };

        if is_https {
            let tls_stream = match self.tls.connect(&host, tcp_stream).await {
                Ok(s) => s,
                Err(_) => return,
            };
            let negotiated = TlsManager::negotiated_alpn(tls_stream.get_ref().1.alpn_protocol());
            let io = TokioIo::new(tls_stream);

            if negotiated == NegotiatedProtocol::H2 {
                if let Ok((sender, conn)) = hyper::client::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .initial_stream_window_size(Some(2 * 1024 * 1024))
                    .initial_connection_window_size(Some(4 * 1024 * 1024))
                    .handshake(io)
                    .await
                {
                    tokio::spawn(async move {
                        let _ = conn.await;
                    });
                    self.pool.release(pool_key, PooledSender::Http2(sender)).await;
                }
            } else if let Ok((sender, conn)) = hyper::client::conn::http1::handshake(io).await {
                tokio::spawn(async move {
                    let _ = conn.await;
                });
                self.pool.release(pool_key, PooledSender::Http1(sender)).await;
            }
        } else {
            let io = TokioIo::new(tcp_stream);
            if let Ok((sender, conn)) = hyper::client::conn::http1::handshake(io).await {
                tokio::spawn(async move {
                    let _ = conn.await;
                });
                self.pool.release(pool_key, PooledSender::Http1(sender)).await;
            }
        }
    }

    fn request_cache_control(&self) -> crate::cache::CacheControlDirectives {
        if let Some(val) = self.request_headers.get(hyper::header::CACHE_CONTROL) {
            if let Ok(s) = val.to_str() {
                return crate::cache::CacheControlDirectives::parse(s);
            }
        }
        crate::cache::CacheControlDirectives::default()
    }

    pub async fn fetch(&self, url: &RixUrl) -> Result<HttpResponse> {
        self.fetch_with_conditionals(url, &ConditionalRequest::empty())
            .await
    }

    pub async fn request(
        &self,
        url: &RixUrl,
        method: &str,
        headers: &HashMap<String, String>,
        body: Vec<u8>,
    ) -> Result<HttpResponse> {
        let mut request = self.clone();
        request.request_method = Method::from_bytes(method.as_bytes())
            .map_err(|error| NetError::Http(format!("Invalid HTTP method: {error}")))?;
        if matches!(request.request_method, Method::GET | Method::HEAD) && !body.is_empty() {
            return Err(NetError::Http("GET and HEAD cannot carry a body".into()));
        }
        request.request_headers.clear();
        for (name, value) in headers {
            let name = name
                .parse::<hyper::header::HeaderName>()
                .map_err(|error| NetError::Header(error.to_string()))?;
            let value = HeaderValue::from_str(value)
                .map_err(|error| NetError::Header(error.to_string()))?;
            request.request_headers.insert(name, value);
        }
        request.request_body = Bytes::from(body);
        request.fetch(url).await
    }

    pub async fn request_stream(
        &self,
        url: &RixUrl,
        method: &str,
        headers: &HashMap<String, String>,
        body: Vec<u8>,
    ) -> Result<StreamingResponse> {
        let mut request = self.clone();
        request.request_method = Method::from_bytes(method.as_bytes())
            .map_err(|error| NetError::Http(format!("Invalid HTTP method: {error}")))?;
        if matches!(request.request_method, Method::GET | Method::HEAD) && !body.is_empty() {
            return Err(NetError::Http("GET and HEAD cannot carry a body".into()));
        }
        request.request_headers.clear();
        for (name, value) in headers {
            let name = name
                .parse::<hyper::header::HeaderName>()
                .map_err(|error| NetError::Header(error.to_string()))?;
            let value = HeaderValue::from_str(value)
                .map_err(|error| NetError::Header(error.to_string()))?;
            request.request_headers.insert(name, value);
        }
        if matches!(request.request_method, Method::GET) && body.is_empty() {
            if let Some(cache) = &request.http_cache {
                let now = crate::cache::now_secs();
                let url_str = url.as_str();
                if let Some(entry) = cache.get(url_str) {
                    if entry.is_fresh(now) {
                        return Ok(StreamingResponse::from_cached_entry(&entry, url.clone()));
                    }
                }
            }
        }
        request.request_body = Bytes::from(body);
        request.fetch_stream(url).await
    }

    pub async fn request_stream_body(
        &self,
        url: &RixUrl,
        method: &str,
        headers: &HashMap<String, String>,
        body: RequestBodyStream,
    ) -> Result<StreamingResponse> {
        let mut request = self.clone();
        request.request_method = Method::from_bytes(method.as_bytes())
            .map_err(|error| NetError::Http(format!("Invalid HTTP method: {error}")))?;
        if matches!(request.request_method, Method::GET | Method::HEAD) {
            return Err(NetError::Http("GET and HEAD cannot carry a body".into()));
        }
        request.request_headers.clear();
        for (name, value) in headers {
            let name = name
                .parse::<hyper::header::HeaderName>()
                .map_err(|error| NetError::Header(error.to_string()))?;
            let value = HeaderValue::from_str(value)
                .map_err(|error| NetError::Header(error.to_string()))?;
            request.request_headers.insert(name, value);
        }
        request.request_headers.remove("content-length");
        request.request_headers.remove("transfer-encoding");
        request.request_body = Bytes::new();
        tracing::debug!(target: "rix_net::upload", url = %url.as_str(), "streamed upload: resolving the connection");
        request.fetch_stream_with_upload(url, body).await
    }

    pub async fn fetch_stream(&self, url: &RixUrl) -> Result<StreamingResponse> {
        self.follow_stream_redirects(url, None).await
    }

    async fn fetch_stream_with_upload(
        &self,
        url: &RixUrl,
        body: RequestBodyStream,
    ) -> Result<StreamingResponse> {
        self.follow_stream_redirects(url, Some(body)).await
    }

    async fn follow_stream_redirects(
        &self,
        url: &RixUrl,
        upload: Option<RequestBodyStream>,
    ) -> Result<StreamingResponse> {
        let clock = FetchClock::new();
        let mut current_url = url.clone();
        let mut redirect_count = 0;
        let mut redirect_start_ms = None;
        let mut redirect_end_ms = None;
        let mut current_conditional = ConditionalRequest::empty();
        let mut request = self.clone();
        let mut upload = upload;
        let bounded_heads = upload.is_none();

        loop {
            let fetch_start_ms = clock.elapsed_ms();
            let request_start_ms = clock.elapsed_ms();
            let had_body = upload.is_some();
            let exchange = match upload.take() {
                Some(body) => request.upload_head_single(&current_url, body).await?,
                None => {
                    let head = request.fetch_head_single(&current_url, &current_conditional);
                    if bounded_heads {
                        tokio::time::timeout(self.request_timeout, head)
                            .await
                            .map_err(|_| NetError::Timeout(self.request_timeout))??
                    } else {
                        head.await?
                    }
                }
            };
            let response_start_ms = clock.elapsed_ms();
            let host = current_url
                .host_str()
                .ok_or_else(|| NetError::Http("URL missing host".into()))?
                .to_string();
            let status = exchange.parts.status.as_u16();
            let headers = header_map_to_hash_map(&exchange.parts.headers);
            self.observe_alt_svc(&current_url, &headers).await;
            let set_cookies =
                self.collect_set_cookies(&exchange.parts.headers, &current_url, &host);

            if let Some(location) = redirect_location(status, &headers) {
                let next_url = current_url
                    .join(location)
                    .unwrap_or_else(|_| current_url.clone());
                if next_url.as_str() != current_url.as_str() {
                    if redirect_count >= self.max_redirects {
                        return Err(NetError::TooManyRedirects(self.max_redirects));
                    }
                    if !same_origin(&current_url, &next_url) {
                        for header in ["authorization", "proxy-authorization", "cookie"] {
                            request.request_headers.remove(header);
                        }
                    }
                    if (matches!(status, 301 | 302) && request.request_method == Method::POST)
                        || (status == 303
                            && !matches!(request.request_method, Method::GET | Method::HEAD))
                    {
                        request.request_method = Method::GET;
                        request.request_body = Bytes::new();
                        for header in [
                            "content-length",
                            "content-type",
                            "content-encoding",
                            "content-language",
                            "content-location",
                        ] {
                            request.request_headers.remove(header);
                        }
                    } else if had_body {
                        return Err(NetError::Http(format!(
                            "a streaming request body cannot be replayed for HTTP {status} redirect"
                        )));
                    }
                    drop(exchange);
                    redirect_start_ms.get_or_insert(fetch_start_ms);
                    redirect_end_ms = Some(response_start_ms);
                    redirect_count += 1;
                    current_url = next_url;
                    current_conditional = ConditionalRequest::empty();
                    continue;
                }
            }

            let mut timing = clock.finish(fetch_start_ms, request_start_ms, response_start_ms);
            timing.navigation.redirect_start_ms = redirect_start_ms;
            timing.navigation.redirect_end_ms = redirect_end_ms;
            timing.navigation.redirect_count = u32::try_from(redirect_count).unwrap_or(u32::MAX);
            let StreamedExchange {
                body,
                protocol,
                pool_key,
                sender,
                ..
            } = exchange;
            let content_type = headers
                .get("content-type")
                .cloned()
                .unwrap_or_else(|| "text/html; charset=utf-8".to_string());
            let encoding = headers.get("content-encoding").cloned().unwrap_or_default();
            let decoder = StreamingDecoder::for_header(&encoding, Some(self.max_response_bytes))?;
            let release = sender.map(|sender| (self.pool.clone(), pool_key, sender));
            return Ok(StreamingResponse {
                status,
                headers,
                final_url: current_url,
                content_type,
                protocol,
                navigation_timing: timing.navigation,
                navigation_started: timing.started,
                set_cookies,
                body: BodyStream {
                    body: Some(body),
                    decoder,
                    decoded: Vec::new(),
                    release,
                    cached: None,
                    done: false,
                },
            });
        }
    }

    pub async fn fetch_conditional(
        &self,
        url: &RixUrl,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<HttpResponse> {
        let conditional = ConditionalRequest {
            etag: etag.map(str::to_string),
            last_modified: last_modified.map(str::to_string),
        };
        self.fetch_with_conditionals(url, &conditional).await
    }

    async fn fetch_with_conditionals(
        &self,
        url: &RixUrl,
        conditional: &ConditionalRequest,
    ) -> Result<HttpResponse> {
        let is_get_or_head = matches!(self.request_method, Method::GET | Method::HEAD);
        let is_http = matches!(
            url.scheme(),
            rix_core::url::RixScheme::Http | rix_core::url::RixScheme::Https
        );
        let req_cc = self.request_cache_control();

        if let Some(cache) = &self.http_cache {
            if is_get_or_head && is_http && !req_cc.no_store {
                let url_str = url.as_str();
                let now = crate::cache::now_secs();

                if let Some(entry) = cache.get(url_str) {
                    
                    if conditional.is_empty() && !req_cc.no_cache && entry.is_fresh(now) {
                        tracing::debug!(
                            target: "rix_net::cache",
                            url = %url_str,
                            age = entry.meta.current_age(now),
                            "HTTP cache fresh HIT"
                        );
                        return Ok(cache.to_http_response(&entry, now));
                    }

                    if entry.can_revalidate() && conditional.is_empty() {
                        let reval = ConditionalRequest {
                            etag: entry.meta.etag.clone(),
                            last_modified: entry.meta.last_modified.clone(),
                        };
                        tracing::debug!(
                            target: "rix_net::cache",
                            url = %url_str,
                            etag = ?reval.etag,
                            last_modified = ?reval.last_modified,
                            "HTTP cache conditional revalidation"
                        );
                        let response = tokio::time::timeout(
                            self.request_timeout,
                            self.fetch_redirect_chain(url, &reval),
                        )
                        .await
                        .map_err(|_| NetError::Timeout(self.request_timeout))??;

                        if response.status == 304 {
                            tracing::debug!(
                                target: "rix_net::cache",
                                url = %url_str,
                                "HTTP 304 Not Modified, returning cached body"
                            );
                            if let Some(updated) =
                                cache.update_revalidated(url_str, &response.headers, now)
                            {
                                return Ok(cache.to_http_response(&updated, now));
                            } else {
                                return Ok(cache.to_http_response(&entry, now));
                            }
                        } else if response.status == 200 {
                            tracing::debug!(
                                target: "rix_net::cache",
                                url = %url_str,
                                "HTTP 200 OK replacing stale cached response"
                            );
                            let _ = cache.put(url_str, &response);
                            if response.final_url.as_str() != url_str {
                                let _ = cache.put(response.final_url.as_str(), &response);
                            }
                            return Ok(response);
                        } else {
                            return Ok(response);
                        }
                    }
                }

                let response = tokio::time::timeout(
                    self.request_timeout,
                    self.fetch_redirect_chain(url, conditional),
                )
                .await
                .map_err(|_| NetError::Timeout(self.request_timeout))??;

                if response.status == 200 {
                    let _ = cache.put(url_str, &response);
                    if response.final_url.as_str() != url_str {
                        let _ = cache.put(response.final_url.as_str(), &response);
                    }
                }

                return Ok(response);
            }
        }

        tokio::time::timeout(
            self.request_timeout,
            self.fetch_redirect_chain(url, conditional),
        )
        .await
        .map_err(|_| NetError::Timeout(self.request_timeout))?
    }

    async fn fetch_redirect_chain(
        &self,
        url: &RixUrl,
        conditional: &ConditionalRequest,
    ) -> Result<HttpResponse> {
        let clock = FetchClock::new();
        let mut current_url = url.clone();
        let mut redirect_count = 0;
        let mut redirect_start_ms = None;
        let mut redirect_end_ms = None;
        let mut current_conditional = conditional.clone();
        let mut request = self.clone();

        loop {
            let fetch_start_ms = clock.elapsed_ms();
            let mut response = request
                .fetch_single(&current_url, &current_conditional, clock, fetch_start_ms)
                .await?;
            response.navigation_timing.redirect_start_ms = redirect_start_ms;
            response.navigation_timing.redirect_end_ms = redirect_end_ms;
            response.navigation_timing.redirect_count =
                u32::try_from(redirect_count).unwrap_or(u32::MAX);
            self.observe_alt_svc(&current_url, &response.headers).await;

            if [301, 302, 303, 307, 308].contains(&response.status) {
                if redirect_count >= self.max_redirects {
                    return Err(NetError::TooManyRedirects(self.max_redirects));
                }

                if let Some(location) = response.headers.get("location") {
                    let next_url = current_url
                        .join(location)
                        .unwrap_or_else(|_| current_url.clone());
                    if next_url.as_str() == current_url.as_str() && response.status != 304 {
                        return Ok(response);
                    }
                    if !same_origin(&current_url, &next_url) {
                        for header in ["authorization", "proxy-authorization", "cookie"] {
                            request.request_headers.remove(header);
                        }
                    }
                    if (matches!(response.status, 301 | 302)
                        && request.request_method == Method::POST)
                        || (response.status == 303
                            && !matches!(request.request_method, Method::GET | Method::HEAD))
                    {
                        request.request_method = Method::GET;
                        request.request_body = Bytes::new();
                        for header in [
                            "content-length",
                            "content-type",
                            "content-encoding",
                            "content-language",
                            "content-location",
                        ] {
                            request.request_headers.remove(header);
                        }
                    }
                    current_url = next_url;
                    current_conditional = ConditionalRequest::empty();
                    redirect_start_ms.get_or_insert(response.navigation_timing.fetch_start_ms);
                    redirect_end_ms = Some(response.navigation_timing.response_end_ms);
                    redirect_count += 1;
                    continue;
                }
            }

            return Ok(response);
        }
    }

    fn build_request(
        &self,
        url: &RixUrl,
        is_h2: bool,
        conditional: &ConditionalRequest,
    ) -> Result<Request<Full<Bytes>>> {
        Ok(self
            .build_request_head(url, is_h2, conditional)?
            .map(|_| Full::new(self.request_body.clone())))
    }

    fn build_request_head(
        &self,
        url: &RixUrl,
        is_h2: bool,
        conditional: &ConditionalRequest,
    ) -> Result<Request<()>> {
        let is_https = match url.scheme() {
            RixScheme::Https => true,
            RixScheme::Http => false,
            other => return Err(NetError::Http(format!("Unsupported scheme: {other}"))),
        };

        let path = url.path();
        let path_and_query = match url.query() {
            Some(q) => format!("{path}?{q}"),
            None => path.to_string(),
        };
        let authority = request_authority(url)?;

        let mut req_builder = Request::builder().method(self.request_method.clone());

        if is_h2 {
            let scheme_str = if is_https { "https" } else { "http" };
            let full_uri_str = format!("{scheme_str}://{authority}{path_and_query}");
            req_builder = req_builder.uri(full_uri_str);
        } else {
            req_builder = req_builder.uri(path_and_query).header(HOST, authority);
        }

        req_builder = self.apply_common_headers(req_builder, url, conditional)?;

        req_builder
            .body(())
            .map_err(|e| NetError::Http(e.to_string()))
    }

    fn build_upload_request(
        &self,
        url: &RixUrl,
        is_h2: bool,
        conditional: &ConditionalRequest,
        body: RequestBodyStream,
    ) -> Result<Request<RequestBodyStream>> {
        let (parts, _) = self
            .build_request_head(url, is_h2, conditional)?
            .into_parts();
        Ok(Request::from_parts(parts, body))
    }

    fn build_h3_request(
        &self,
        url: &RixUrl,
        conditional: &ConditionalRequest,
    ) -> Result<http::Request<()>> {
        match url.scheme() {
            RixScheme::Https => {}
            other => return Err(NetError::Http(format!("H3 requires https, got: {other}"))),
        }

        let path = url.path();
        let path_and_query = match url.query() {
            Some(q) => format!("{path}?{q}"),
            None => path.to_string(),
        };
        let authority = request_authority(url)?;
        let full_uri_str = format!("https://{authority}{path_and_query}");

        let builder = http::Request::builder()
            .method(self.request_method.clone())
            .uri(full_uri_str);
        self.apply_common_headers(builder, url, conditional)
            .map(|b| b.body(()).map_err(|e| NetError::Http(e.to_string())))?
    }

    fn apply_common_headers(
        &self,
        mut builder: http::request::Builder,
        url: &RixUrl,
        conditional: &ConditionalRequest,
    ) -> Result<http::request::Builder> {
        builder = builder
            .header(USER_AGENT, &self.user_agent)
            .header("accept-language", &self.accept_language)
            .header(
                ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
            )
            .header(ACCEPT_ENCODING, "gzip, deflate, br");

        if let Some(etag) = &conditional.etag {
            if !etag.is_empty() {
                builder = builder.header("if-none-match", etag.as_str());
            }
        }
        if let Some(last_modified) = &conditional.last_modified {
            if !last_modified.is_empty() {
                builder = builder.header("if-modified-since", last_modified.as_str());
            }
        }

        if let Some(storage) = &self.storage {
            if let Ok(cookies) = storage.get_cookies_for_url(url) {
                if !cookies.is_empty() {
                    let cookie_str = cookies
                        .iter()
                        .map(|c| format!("{}={}", c.name, c.value))
                        .collect::<Vec<_>>()
                        .join("; ");
                    if let Ok(val) = HeaderValue::from_str(&cookie_str) {
                        builder = builder.header(COOKIE, val);
                    }
                }
            }
        }

        if let Some(headers) = builder.headers_mut() {
            for (name, value) in &self.request_headers {
                headers.insert(name, value.clone());
            }
            if !self.request_body.is_empty() {
                headers.insert(
                    hyper::header::CONTENT_LENGTH,
                    HeaderValue::from(self.request_body.len()),
                );
            }
            if !headers.contains_key("sec-fetch-site") {
                headers.insert(
                    hyper::header::HeaderName::from_static("sec-fetch-site"),
                    HeaderValue::from_static("none"),
                );
            }
            if !headers.contains_key("sec-fetch-mode") {
                headers.insert(
                    hyper::header::HeaderName::from_static("sec-fetch-mode"),
                    HeaderValue::from_static("navigate"),
                );
            }
            if !headers.contains_key("sec-fetch-dest") {
                headers.insert(
                    hyper::header::HeaderName::from_static("sec-fetch-dest"),
                    HeaderValue::from_static("document"),
                );
            }
            if !headers.contains_key("sec-fetch-user") {
                headers.insert(
                    hyper::header::HeaderName::from_static("sec-fetch-user"),
                    HeaderValue::from_static("?1"),
                );
            }
        }
        Ok(builder)
    }

    async fn fetch_single(
        &self,
        url: &RixUrl,
        conditional: &ConditionalRequest,
        clock: FetchClock,
        fetch_start_ms: f64,
    ) -> Result<HttpResponse> {
        let is_https = match url.scheme() {
            RixScheme::Https => true,
            RixScheme::Http => false,
            other => return Err(NetError::Http(format!("Unsupported scheme: {other}"))),
        };

        if is_https && matches!(self.request_method, Method::GET | Method::HEAD) {
            if let Some(port) = self.advertised_h3_port(url).await {
                let h3 = tokio::time::timeout(
                    H3_PROBE_TIMEOUT,
                    self.fetch_h3(url, conditional, port, clock, fetch_start_ms),
                );
                let tcp = self.fetch_tcp(url, conditional, clock, fetch_start_ms);
                tokio::pin!(h3, tcp);
                tokio::select! {
                    result = &mut h3 => {
                        match result {
                            Ok(Ok(response)) => {
                                self.mark_h3(url, H3OriginState::Supported(port)).await;
                                return Ok(response);
                            }
                            failure => {
                                tracing::debug!(?failure, "Advertised H3 failed; awaiting TCP");
                                self.mark_h3(url, H3OriginState::Unsupported).await;
                                return tcp.await;
                            }
                        }
                    }
                    result = &mut tcp => {
                        match result {
                            Ok(response) => return Ok(response),
                            Err(tcp_error) => {
                                return match h3.await {
                                    Ok(Ok(response)) => {
                                        self.mark_h3(url, H3OriginState::Supported(port)).await;
                                        Ok(response)
                                    }
                                    _ => {
                                        self.mark_h3(url, H3OriginState::Unsupported).await;
                                        Err(tcp_error)
                                    }
                                };
                            }
                        }
                    }
                }
            }
        }

        self.fetch_tcp(url, conditional, clock, fetch_start_ms)
            .await
    }

    fn origin_key(url: &RixUrl) -> String {
        let host = url.host_str().unwrap_or("");
        let port = url.port().unwrap_or(443);
        format!("{host}:{port}")
    }

    async fn advertised_h3_port(&self, url: &RixUrl) -> Option<u16> {
        self.quinn_endpoint.as_ref()?;
        match self.h3_state.lock().await.get(&Self::origin_key(url)) {
            Some(H3OriginState::Advertised(port) | H3OriginState::Supported(port)) => Some(*port),
            _ => None,
        }
    }

    async fn observe_alt_svc(&self, url: &RixUrl, headers: &HashMap<String, String>) {
        if url.scheme() != RixScheme::Https {
            return;
        }
        let Some(value) = headers.get("alt-svc") else {
            return;
        };
        let key = Self::origin_key(url);
        let mut state = self.h3_state.lock().await;
        if value.trim().eq_ignore_ascii_case("clear") {
            state.remove(&key);
        } else if let Some(port) = advertised_same_host_h3_port(value) {
            state.entry(key).or_insert(H3OriginState::Advertised(port));
        }
    }

    async fn mark_h3(&self, url: &RixUrl, state: H3OriginState) {
        let key = Self::origin_key(url);
        let mut guard = self.h3_state.lock().await;
        guard.insert(key, state);
    }

    async fn fetch_h3(
        &self,
        url: &RixUrl,
        conditional: &ConditionalRequest,
        port: u16,
        clock: FetchClock,
        fetch_start_ms: f64,
    ) -> Result<HttpResponse> {
        let host = url
            .host_str()
            .ok_or_else(|| NetError::Http("URL missing host".into()))?
            .to_string();
        let pool_key = format!("https:{host}:{port}");

        if let Some(mut pooled) = self.pool.acquire_h3(&pool_key).await {
            match self
                .h3_exchange(&mut pooled, url, conditional, clock, fetch_start_ms)
                .await
            {
                Ok(response) => {
                    self.pool
                        .release(pool_key, PooledSender::Http3(pooled))
                        .await;
                    return Ok(response);
                }
                Err(e) => {
                    tracing::debug!("Pooled H3 exchange failed: {e}; opening new QUIC connection");
                }
            }
        }

        let endpoint = self.quinn_endpoint.clone().ok_or_else(|| {
            NetError::Quic("QUIC endpoint unavailable (UDP blocked or bind failed)".into())
        })?;
        let addrs = self.dns.resolve_socket_addrs(&host, port).await?;
        let mut last_err = String::from("no addresses resolved");

        for addr in addrs {
            let connecting = endpoint
                .connect(addr, host.as_str())
                .map_err(|e| NetError::Quic(format!("QUIC connect to {addr} failed: {e}")))?;
            let quinn_conn = connecting
                .await
                .map_err(|e| NetError::Quic(format!("QUIC handshake with {addr} failed: {e}")))?;

            let h3_conn = h3_quinn::Connection::new(quinn_conn);
            let (mut driver, mut sender) = h3::client::new(h3_conn)
                .await
                .map_err(|e| NetError::H3(format!("H3 setup with {addr} failed: {e}")))?;
            tokio::spawn(async move {
                let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
            });

            match self
                .h3_exchange(&mut sender, url, conditional, clock, fetch_start_ms)
                .await
            {
                Ok(response) => {
                    self.pool
                        .release(pool_key.clone(), PooledSender::Http3(sender))
                        .await;
                    return Ok(response);
                }
                Err(e) => {
                    last_err = format!("H3 exchange with {addr} failed: {e}");
                    continue;
                }
            }
        }

        Err(NetError::H3(last_err))
    }

    async fn h3_exchange(
        &self,
        sender: &mut H3Sender,
        url: &RixUrl,
        conditional: &ConditionalRequest,
        clock: FetchClock,
        fetch_start_ms: f64,
    ) -> Result<HttpResponse> {
        let request = self.build_h3_request(url, conditional)?;
        let request_start_ms = clock.elapsed_ms();
        let mut stream = sender
            .send_request(request)
            .await
            .map_err(|e| NetError::H3(format!("H3 send_request failed: {e}")))?;
        if !self.request_body.is_empty() {
            stream
                .send_data(self.request_body.clone())
                .await
                .map_err(|error| NetError::H3(format!("H3 send_data failed: {error}")))?;
        }
        stream
            .finish()
            .await
            .map_err(|e| NetError::H3(format!("H3 finish failed: {e}")))?;

        let response = stream
            .recv_response()
            .await
            .map_err(|e| NetError::H3(format!("H3 recv_response failed: {e}")))?;
        let response_start_ms = clock.elapsed_ms();
        let status = response.status().as_u16();
        let mut headers = HashMap::new();
        for (key, value) in response.headers().iter() {
            if let Ok(val_str) = value.to_str() {
                headers.insert(key.as_str().to_lowercase(), val_str.to_string());
            }
        }

        let mut raw: Vec<u8> = Vec::new();
        while let Some(chunk) = stream
            .recv_data()
            .await
            .map_err(|e| NetError::H3(format!("H3 recv_data failed: {e}")))?
        {
            let mut chunk = chunk;
            if raw.len().saturating_add(chunk.remaining()) > self.max_response_bytes {
                return Err(NetError::ResponseTooLarge(self.max_response_bytes));
            }
            while chunk.has_remaining() {
                let bytes = chunk.chunk();
                raw.extend_from_slice(bytes);
                let consumed = bytes.len();
                chunk.advance(consumed);
            }
        }

        let host = url.host_str().unwrap_or("");
        let timing = clock.finish(fetch_start_ms, request_start_ms, response_start_ms);
        self.finalize_response(
            status,
            &headers,
            &raw,
            url,
            host,
            NegotiatedProtocol::H3,
            timing,
        )
    }

    async fn fetch_tcp(
        &self,
        url: &RixUrl,
        conditional: &ConditionalRequest,
        clock: FetchClock,
        fetch_start_ms: f64,
    ) -> Result<HttpResponse> {
        let host = url
            .host_str()
            .ok_or_else(|| NetError::Http("URL missing host".into()))?
            .to_string();

        let request_start_ms = clock.elapsed_ms();
        let exchange = self.fetch_head_single(url, conditional).await?;
        let response_start_ms = clock.elapsed_ms();

        let StreamedExchange {
            parts,
            body,
            protocol,
            pool_key,
            sender,
        } = exchange;
        let collected = collect_body_limited(body, self.max_response_bytes).await?;
        let timing = clock.finish(fetch_start_ms, request_start_ms, response_start_ms);
        if let Some(sender) = sender {
            return_sender(&self.pool, &pool_key, sender).await;
        }
        self.response_from_hyper_parts(
            parts.status.as_u16(),
            &parts.headers,
            &collected,
            url,
            &host,
            protocol,
            timing,
        )
    }

    async fn fetch_head_single(
        &self,
        url: &RixUrl,
        conditional: &ConditionalRequest,
    ) -> Result<StreamedExchange> {
        let host = url
            .host_str()
            .ok_or_else(|| NetError::Http("URL missing host".into()))?
            .to_string();

        let is_https = match url.scheme() {
            RixScheme::Https => true,
            RixScheme::Http => false,
            other => return Err(NetError::Http(format!("Unsupported scheme: {other}"))),
        };

        let default_port = if is_https { 443 } else { 80 };
        let port = url.port().unwrap_or(default_port);
        let pool_key = format!("{}:{host}:{port}", if is_https { "https" } else { "http" });

        if let Some(pooled) = self.pool.acquire_tcp(&pool_key).await {
            tracing::debug!(target: "rix_net::exchange", host = %host, "buffered request: reusing a pooled connection");
            match pooled {
                PooledSender::Http1(mut h1) => {
                    let request = self.build_request(url, false, conditional)?;
                    if let Ok(response) = h1.send_request(request).await {
                        let (parts, body) = response.into_parts();
                        return Ok(StreamedExchange {
                            parts,
                            body,
                            protocol: NegotiatedProtocol::Http11,
                            pool_key,
                            sender: Some(PooledSender::Http1(h1)),
                        });
                    }
                }
                PooledSender::Http2(mut h2) => {
                    let request = self.build_request(url, true, conditional)?;
                    if let Ok(response) = h2.send_request(request).await {
                        let (parts, body) = response.into_parts();
                        return Ok(StreamedExchange {
                            parts,
                            body,
                            protocol: NegotiatedProtocol::H2,
                            pool_key,
                            sender: None,
                        });
                    }
                }
                PooledSender::Http3(h3) => {
                    self.pool
                        .release(pool_key.clone(), PooledSender::Http3(h3))
                        .await;
                }
            }
        }

        let mut addrs = self.dns.resolve_socket_addrs(&host, port).await?;
        tracing::debug!(
            target: "rix_net::exchange",
            host = %host,
            port,
            addresses = addrs.len(),
            "buffered request: dialing"
        );
        let mut last_err = NetError::Http("no addresses resolved".into());
        while !addrs.is_empty() {
            let (tcp_stream, connected_address) =
                connect_happy_eyeballs(&addrs, HAPPY_EYEBALLS_DELAY, TCP_CONNECT_TIMEOUT).await?;
            tracing::debug!(
                target: "rix_net::exchange",
                host = %host,
                %connected_address,
                "buffered request: tcp connected"
            );
            addrs.retain(|address| *address != connected_address);
            match self
                .serve_head_over_tcp_stream(
                    tcp_stream,
                    url,
                    &host,
                    is_https,
                    &pool_key,
                    conditional,
                )
                .await
            {
                Ok(exchange) => return Ok(exchange),
                Err(error) => {
                    if !matches!(self.request_method, Method::GET | Method::HEAD) {
                        return Err(error);
                    }
                    last_err = error;
                }
            }
        }
        Err(last_err)
    }

    async fn upload_head_single(
        &self,
        url: &RixUrl,
        body: RequestBodyStream,
    ) -> Result<StreamedExchange> {
        let host = url
            .host_str()
            .ok_or_else(|| NetError::Http("URL missing host".into()))?
            .to_string();
        let is_https = match url.scheme() {
            RixScheme::Https => true,
            RixScheme::Http => false,
            other => return Err(NetError::Http(format!("Unsupported scheme: {other}"))),
        };
        let default_port = if is_https { 443 } else { 80 };
        let port = url.port().unwrap_or(default_port);
        let pool_key = format!("{}:{host}:{port}", if is_https { "https" } else { "http" });

        let addrs = self.dns.resolve_socket_addrs(&host, port).await?;
        let (tcp_stream, _connected) =
            connect_happy_eyeballs(&addrs, HAPPY_EYEBALLS_DELAY, TCP_CONNECT_TIMEOUT).await?;
        self.serve_upload_over_tcp_stream(tcp_stream, url, &host, is_https, &pool_key, body)
            .await
    }

    async fn serve_upload_over_tcp_stream(
        &self,
        tcp_stream: TcpStream,
        url: &RixUrl,
        host: &str,
        is_https: bool,
        pool_key: &str,
        body: RequestBodyStream,
    ) -> Result<StreamedExchange> {
        let conditional = ConditionalRequest::empty();
        if is_https {
            let tls_stream = self.tls.connect(host, tcp_stream).await?;
            let negotiated = TlsManager::negotiated_alpn(tls_stream.get_ref().1.alpn_protocol());
            let io = TokioIo::new(tls_stream);

            if negotiated == NegotiatedProtocol::H2 {
                let (mut sender, conn) = hyper::client::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .initial_stream_window_size(Some(2 * 1024 * 1024))
                    .initial_connection_window_size(Some(4 * 1024 * 1024))
                    .handshake(io)
                    .await?;
                tokio::spawn(async move {
                    let _ = conn.await;
                });

                let request = self.build_upload_request(url, true, &conditional, body)?;
                tracing::debug!(target: "rix_net::upload", host = %host, protocol = "h2", "sending a streaming upload request");
                let response = sender.send_request(request).await?;
                tracing::debug!(target: "rix_net::upload", status = response.status().as_u16(), "streaming upload response head received");
                let (parts, body) = response.into_parts();
                Ok(StreamedExchange {
                    parts,
                    body,
                    protocol: NegotiatedProtocol::H2,
                    pool_key: pool_key.to_string(),
                    sender: None,
                })
            } else {
                let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
                tokio::spawn(async move {
                    let _ = conn.await;
                });

                let request = self.build_upload_request(url, false, &conditional, body)?;
                tracing::debug!(target: "rix_net::upload", host = %host, protocol = "http/1.1", "sending a streaming upload request");
                let response = sender.send_request(request).await?;
                tracing::debug!(target: "rix_net::upload", status = response.status().as_u16(), "streaming upload response head received");
                let (parts, body) = response.into_parts();
                Ok(StreamedExchange {
                    parts,
                    body,
                    protocol: NegotiatedProtocol::Http11,
                    pool_key: pool_key.to_string(),
                    sender: None,
                })
            }
        } else {
            let io = TokioIo::new(tcp_stream);
            let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
            tokio::spawn(async move {
                let _ = conn.await;
            });

            let request = self.build_upload_request(url, false, &conditional, body)?;
            tracing::debug!(target: "rix_net::upload", host = %host, protocol = "http/1.1", "sending a streaming upload request");
            let response = sender.send_request(request).await?;
            tracing::debug!(target: "rix_net::upload", status = response.status().as_u16(), "streaming upload response head received");
            let (parts, body) = response.into_parts();
            Ok(StreamedExchange {
                parts,
                body,
                protocol: NegotiatedProtocol::Http11,
                pool_key: pool_key.to_string(),
                sender: None,
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn serve_head_over_tcp_stream(
        &self,
        tcp_stream: TcpStream,
        url: &RixUrl,
        host: &str,
        is_https: bool,
        pool_key: &str,
        conditional: &ConditionalRequest,
    ) -> Result<StreamedExchange> {
        if is_https {
            tracing::debug!(target: "rix_net::exchange", %host, "buffered request: tls handshake");
            let tls_stream = self.tls.connect(host, tcp_stream).await?;
            let negotiated = TlsManager::negotiated_alpn(tls_stream.get_ref().1.alpn_protocol());
            tracing::debug!(
                target: "rix_net::exchange",
                %host,
                ?negotiated,
                "buffered request: tls negotiated"
            );
            let io = TokioIo::new(tls_stream);

            if negotiated == NegotiatedProtocol::H2 {
                let (mut sender, conn) = hyper::client::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .initial_stream_window_size(Some(2 * 1024 * 1024))
                    .initial_connection_window_size(Some(4 * 1024 * 1024))
                    .handshake(io)
                    .await?;
                tokio::spawn(async move {
                    let _ = conn.await;
                });
                tracing::debug!(target: "rix_net::exchange", %host, "buffered request: h2 ready");
                self.pool
                    .release(pool_key.to_string(), PooledSender::Http2(sender.clone()))
                    .await;

                let request = self.build_request(url, true, conditional)?;
                let response = sender.send_request(request).await?;
                tracing::debug!(
                    target: "rix_net::exchange",
                    %host,
                    status = response.status().as_u16(),
                    "buffered request: response head"
                );
                let (parts, body) = response.into_parts();
                Ok(StreamedExchange {
                    parts,
                    body,
                    protocol: NegotiatedProtocol::H2,
                    pool_key: pool_key.to_string(),
                    sender: None,
                })
            } else {
                let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
                tokio::spawn(async move {
                    let _ = conn.await;
                });

                let request = self.build_request(url, false, conditional)?;
                let response = sender.send_request(request).await?;
                let (parts, body) = response.into_parts();
                Ok(StreamedExchange {
                    parts,
                    body,
                    protocol: NegotiatedProtocol::Http11,
                    pool_key: pool_key.to_string(),
                    sender: Some(PooledSender::Http1(sender)),
                })
            }
        } else {
            let io = TokioIo::new(tcp_stream);
            let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
            tokio::spawn(async move {
                let _ = conn.await;
            });

            let request = self.build_request(url, false, conditional)?;
            let response = sender.send_request(request).await?;
            let (parts, body) = response.into_parts();
            Ok(StreamedExchange {
                parts,
                body,
                protocol: NegotiatedProtocol::Http11,
                pool_key: pool_key.to_string(),
                sender: Some(PooledSender::Http1(sender)),
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn response_from_hyper_parts(
        &self,
        status: u16,
        header_map: &HeaderMap,
        raw_bytes: &[u8],
        url: &RixUrl,
        host: &str,
        protocol: NegotiatedProtocol,
        timing: CompletedTiming,
    ) -> Result<HttpResponse> {
        let headers = header_map_to_hash_map(header_map);
        let set_cookies = self.collect_set_cookies(header_map, url, host);

        let encoding_str = headers.get("content-encoding").cloned().unwrap_or_default();
        let body =
            decompress_body_for_header_limited(raw_bytes, &encoding_str, self.max_response_bytes)?;

        let content_type = headers
            .get("content-type")
            .cloned()
            .unwrap_or_else(|| "text/html; charset=utf-8".to_string());

        Ok(HttpResponse {
            status,
            headers,
            body,
            final_url: url.clone(),
            content_type,
            protocol,
            navigation_timing: timing.navigation,
            navigation_started: timing.started,
            set_cookies,
        })
    }

    fn collect_set_cookies(&self, header_map: &HeaderMap, url: &RixUrl, host: &str) -> Vec<String> {
        let mut set_cookies = Vec::new();
        for val in header_map.get_all(SET_COOKIE) {
            if let Ok(val_str) = val.to_str() {
                set_cookies.push(val_str.to_string());
            }
        }
        self.process_set_cookie_headers(header_map, url, host);
        set_cookies
    }

    #[allow(clippy::too_many_arguments)]
    fn finalize_response(
        &self,
        status: u16,
        headers: &HashMap<String, String>,
        raw_bytes: &[u8],
        url: &RixUrl,
        host: &str,
        protocol: NegotiatedProtocol,
        timing: CompletedTiming,
    ) -> Result<HttpResponse> {
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            if let (Ok(name), Ok(val)) = (
                k.parse::<hyper::header::HeaderName>(),
                HeaderValue::from_str(v),
            ) {
                map.append(name, val);
            }
        }
        let mut set_cookies = Vec::new();
        for val in map.get_all(SET_COOKIE) {
            if let Ok(val_str) = val.to_str() {
                set_cookies.push(val_str.to_string());
            }
        }
        self.process_set_cookie_headers(&map, url, host);

        let encoding_str = headers.get("content-encoding").cloned().unwrap_or_default();
        let body =
            decompress_body_for_header_limited(raw_bytes, &encoding_str, self.max_response_bytes)?;

        let content_type = headers
            .get("content-type")
            .cloned()
            .unwrap_or_else(|| "text/html; charset=utf-8".to_string());

        Ok(HttpResponse {
            status,
            headers: headers.clone(),
            body,
            final_url: url.clone(),
            content_type,
            protocol,
            navigation_timing: timing.navigation,
            navigation_started: timing.started,
            set_cookies,
        })
    }

    fn process_set_cookie_headers(&self, headers: &HeaderMap, url: &RixUrl, _host: &str) {
        let storage = match &self.storage {
            Some(s) => s,
            None => return,
        };

        for val in headers.get_all(SET_COOKIE) {
            if let Ok(cookie_str) = val.to_str() {
                if let Some(cookie) = parse_cookie_header_for_url(cookie_str, url) {
                    if storage
                        .set_cookie_from_response(
                            cookie,
                            url.scheme() == rix_core::url::RixScheme::Https,
                        )
                        .is_err()
                    {
                        tracing::warn!(
                            "Could not persist a protected cookie; the in-memory session remains available"
                        );
                    }
                }
            }
        }
    }
}

fn request_authority(url: &RixUrl) -> Result<String> {
    let host = url
        .host_str()
        .ok_or_else(|| NetError::Http("URL missing host".into()))?;
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

pub fn same_origin(first: &RixUrl, second: &RixUrl) -> bool {
    let port = |url: &RixUrl| {
        url.port().unwrap_or(if url.scheme() == RixScheme::Https {
            443
        } else {
            80
        })
    };
    first.scheme() == second.scheme()
        && first.host_str() == second.host_str()
        && port(first) == port(second)
}

pub fn compute_sec_fetch_site(initiator: Option<&RixUrl>, target: &RixUrl) -> &'static str {
    let initiator = match initiator {
        Some(url) => url,
        None => return "none",
    };
    if same_origin(initiator, target) {
        return "same-origin";
    }
    let init_host = match initiator.host_str() {
        Some(h) => h,
        None => return "cross-site",
    };
    let target_host = match target.host_str() {
        Some(h) => h,
        None => return "cross-site",
    };
    let init_domain = psl::domain_str(init_host).unwrap_or(init_host);
    let target_domain = psl::domain_str(target_host).unwrap_or(target_host);
    if init_domain.eq_ignore_ascii_case(target_domain) {
        "same-site"
    } else {
        "cross-site"
    }
}

fn advertised_same_host_h3_port(header: &str) -> Option<u16> {
    header.split(',').find_map(|alternative| {
        let (protocol, parameters) = alternative.trim().split_once('=')?;
        if protocol.trim() != "h3" {
            return None;
        }
        let authority = parameters.split(';').next()?.trim();
        let port = authority
            .strip_prefix('"')?
            .strip_suffix('"')?
            .strip_prefix(':')?;
        port.parse().ok().filter(|port| *port != 0)
    })
}

async fn collect_body_limited(mut body: hyper::body::Incoming, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame?.into_data() {
            if bytes.len().saturating_add(data.len()) > limit {
                return Err(NetError::ResponseTooLarge(limit));
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(bytes)
}

fn header_map_to_hash_map(header_map: &HeaderMap) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for (key, value) in header_map.iter() {
        if let Ok(val_str) = value.to_str() {
            headers.insert(key.as_str().to_lowercase(), val_str.to_string());
        }
    }
    headers
}

async fn return_sender(pool: &ConnectionPool, key: &str, sender: PooledSender) {
    match sender {
        PooledSender::Http1(sender) => {
            if sender.is_ready() {
                pool.release(key.to_string(), PooledSender::Http1(sender))
                    .await;
            }
        }
        other => pool.release(key.to_string(), other).await,
    }
}

fn redirect_location<'a>(status: u16, headers: &'a HashMap<String, String>) -> Option<&'a str> {
    if [301, 302, 303, 307, 308].contains(&status) {
        headers.get("location").map(String::as_str)
    } else {
        None
    }
}

fn interleave_address_families(addresses: &[SocketAddr]) -> Vec<SocketAddr> {
    let prefer_ipv6 = addresses.first().is_some_and(SocketAddr::is_ipv6);
    let mut preferred = addresses
        .iter()
        .copied()
        .filter(|address| address.is_ipv6() == prefer_ipv6);
    let mut alternative = addresses
        .iter()
        .copied()
        .filter(|address| address.is_ipv6() != prefer_ipv6);
    let mut interleaved = Vec::with_capacity(addresses.len());
    loop {
        let first = preferred.next();
        let second = alternative.next();
        if first.is_none() && second.is_none() {
            break;
        }
        interleaved.extend(first);
        interleaved.extend(second);
    }
    interleaved
}

async fn connect_happy_eyeballs(
    addresses: &[SocketAddr],
    stagger: Duration,
    timeout: Duration,
) -> Result<(TcpStream, SocketAddr)> {
    let mut attempts = tokio::task::JoinSet::new();
    for (index, address) in interleave_address_families(addresses)
        .into_iter()
        .enumerate()
    {
        attempts.spawn(async move {
            if index > 0 {
                tokio::time::sleep(stagger.saturating_mul(index as u32)).await;
            }
            let stream = tokio::time::timeout(timeout, TcpStream::connect(address))
                .await
                .map_err(|_| NetError::Timeout(timeout))??;
            let _ = stream.set_nodelay(true);
            Ok::<_, NetError>((stream, address))
        });
    }
    let mut last_error = NetError::Http("no addresses resolved".into());
    while let Some(result) = attempts.join_next().await {
        match result {
            Ok(Ok(connection)) => return Ok(connection),
            Ok(Err(error)) => last_error = error,
            Err(error) => last_error = NetError::Http(format!("TCP connect task failed: {error}")),
        }
    }
    Err(last_error)
}

fn create_quic_endpoint(tls: &TlsManager) -> Option<quinn::Endpoint> {
    let candidates: [std::net::SocketAddr; 2] = [
        "[::]:0".parse().expect("IPv6 unspecified is valid"),
        "0.0.0.0:0".parse().expect("IPv4 unspecified is valid"),
    ];
    for bind_addr in candidates {
        match quinn::Endpoint::client(bind_addr) {
            Ok(mut endpoint) => {
                endpoint.set_default_client_config(tls.quinn_client_config());
                tracing::debug!("QUIC endpoint bound to {bind_addr}");
                return Some(endpoint);
            }
            Err(e) => {
                tracing::debug!("QUIC bind to {bind_addr} failed: {e}");
                continue;
            }
        }
    }
    tracing::warn!("QUIC unavailable: UDP bind failed; HTTP/3 disabled, using H2/H1");
    None
}

fn parse_cookie_date(value: &str) -> Option<u64> {
    fn is_delimiter(byte: u8) -> bool {
        matches!(byte, 0x09 | 0x20..=0x2f | 0x3b..=0x40 | 0x5b..=0x60 | 0x7b..=0x7e)
    }

    fn month_number(token: &str) -> Option<u64> {
        let name = token.get(..3)?.to_ascii_lowercase();
        Some(match name.as_str() {
            "jan" => 1,
            "feb" => 2,
            "mar" => 3,
            "apr" => 4,
            "may" => 5,
            "jun" => 6,
            "jul" => 7,
            "aug" => 8,
            "sep" => 9,
            "oct" => 10,
            "nov" => 11,
            "dec" => 12,
            _ => return None,
        })
    }

    fn clock(token: &str) -> Option<(u64, u64, u64)> {
        let mut parts = token.split(':');
        let hour = parts.next()?.parse::<u64>().ok()?;
        let minute = parts.next()?.parse::<u64>().ok()?;
        let second = parts
            .next()
            .map_or(Some(0), |part| part.parse::<u64>().ok())?;
        if parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
            return None;
        }
        Some((hour, minute, second))
    }

    fn days_in_month(year: u64, month: u64) -> u64 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
            _ => 28,
        }
    }

    fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
        let year = if month <= 2 { year - 1 } else { year };
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    let mut hour = None;
    let mut minute = None;
    let mut second = None;
    let mut day = None;
    let mut month = None;
    let mut year = None;

    let bytes = value.as_bytes();
    let mut start = 0;
    for index in 0..=bytes.len() {
        if index < bytes.len() && !is_delimiter(bytes[index]) {
            continue;
        }
        if start < index {
            let token = &value[start..index];
            if token.contains(':') {
                let Some((h, m, s)) = clock(token) else {
                    return None;
                };
                if hour.is_some() {
                    return None;
                }
                hour = Some(h);
                minute = Some(m);
                second = Some(s);
            } else if let Some(number) = month_number(token) {
                if month.is_some() {
                    return None;
                }
                month = Some(number);
            } else if token.bytes().all(|byte| byte.is_ascii_digit()) {
                let parsed = token.parse::<u32>().ok()?;
                match token.len() {
                    1 | 2 => {
                        if day.is_none() && (1..=31).contains(&parsed) {
                            day = Some(parsed as u64);
                        } else if year.is_none() {
                            year = Some(if parsed >= 70 {
                                parsed as u64 + 1900
                            } else {
                                parsed as u64 + 2000
                            });
                        } else {
                            return None;
                        }
                    }
                    3 | 4 => {
                        if year.is_some() {
                            return None;
                        }
                        year = Some(parsed as u64);
                    }
                    _ => {}
                }
            }
        }
        start = index + 1;
    }

    let day = day?;
    let month = month?;
    let year = year?;
    if year < 1601 || day > days_in_month(year, month) {
        return None;
    }
    let days = days_from_civil(year as i64, month as i64, day as i64);
    if days < 0 {
        return Some(0);
    }
    let hour = hour.unwrap_or(0);
    let minute = minute.unwrap_or(0);
    let second = second.unwrap_or(0);
    Some((days as u64) * 86_400 + hour * 3_600 + minute * 60 + second)
}

pub fn parse_cookie_header(cookie_str: &str, default_domain: &str) -> Option<Cookie> {
    if cookie_str.len() > 4096 {
        return None;
    }
    let mut parts = cookie_str.split(';');
    let name_value = parts.next()?.trim();
    let (name, value) = name_value.split_once('=')?;

    let default_domain = default_domain.to_ascii_lowercase();
    let mut domain = default_domain.clone();
    let mut has_domain_attribute = false;
    let mut has_explicit_path = false;
    let mut path = "/".to_string();
    let mut secure = false;
    let mut http_only = false;
    let mut same_site = SameSitePolicy::Lax;
    let mut expires_at: Option<u64> = None;
    let mut max_age_expiry: Option<u64> = None;

    for part in parts {
        let trimmed = part.trim();
        if trimmed.eq_ignore_ascii_case("secure") {
            secure = true;
        } else if trimmed.eq_ignore_ascii_case("httponly") {
            http_only = true;
        } else if let Some((k, v)) = trimmed.split_once('=') {
            let key = k.trim().to_lowercase();
            let val = v.trim();
            match key.as_str() {
                "domain" => {
                    has_domain_attribute = true;
                    let candidate = val.trim_start_matches('.').to_ascii_lowercase();
                    if candidate.is_empty()
                        || candidate.ends_with('.')
                        || candidate.parse::<std::net::IpAddr>().is_ok()
                        || !(default_domain == candidate
                            || default_domain.ends_with(&format!(".{candidate}")))
                    {
                        return None;
                    }
                    if psl::suffix_str(&candidate) == Some(candidate.as_str()) {
                        if candidate != default_domain {
                            return None;
                        }
                        domain = default_domain.clone();
                    } else {
                        domain = format!(".{candidate}");
                    }
                }
                "path" => {
                    if val.starts_with('/') {
                        path = val.to_string();
                        has_explicit_path = true;
                    }
                }
                "max-age" => {
                    if let Ok(secs) = val.parse::<i64>() {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        max_age_expiry = Some(if secs <= 0 {
                            0
                        } else {
                            now.saturating_add(secs as u64)
                        });
                    }
                }
                "expires" => {
                    if let Some(timestamp) = parse_cookie_date(val) {
                        expires_at = Some(timestamp);
                    }
                }
                "samesite" => {
                    same_site = match val.to_lowercase().as_str() {
                        "strict" => SameSitePolicy::Strict,
                        "none" => SameSitePolicy::None,
                        _ => SameSitePolicy::Lax,
                    };
                }
                _ => {}
            }
        }
    }

    let name = name.trim();
    let value = value.trim();
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !b"()<>@,;:\"/[]?={} \t".contains(&byte))
        || value.bytes().any(|byte| byte.is_ascii_control())
        || (name.starts_with("__Secure-") && !secure)
        || (name.starts_with("__Host-")
            && (!secure || has_domain_attribute || !has_explicit_path || path != "/"))
    {
        return None;
    }
    if max_age_expiry.is_some() {
        expires_at = max_age_expiry;
    }

    Some(Cookie {
        name: name.to_string(),
        value: value.to_string(),
        domain,
        path,
        expires_at,
        secure,
        http_only,
        same_site,
    })
}

pub fn parse_cookie_header_for_url(cookie_str: &str, url: &RixUrl) -> Option<Cookie> {
    let host = url.host_str()?;
    let mut cookie = parse_cookie_header(cookie_str, host)?;
    let explicit_path = cookie_str.split(';').skip(1).any(|attribute| {
        attribute
            .trim()
            .split_once('=')
            .is_some_and(|(name, value)| {
                name.trim().eq_ignore_ascii_case("path") && value.trim().starts_with('/')
            })
    });
    if !explicit_path {
        cookie.path = url
            .path()
            .rsplit_once('/')
            .map(
                |(directory, _)| {
                    if directory.is_empty() { "/" } else { directory }
                },
            )
            .unwrap_or("/")
            .to_owned();
    }
    if cookie.secure && url.scheme() != rix_core::url::RixScheme::Https {
        return None;
    }
    Some(cookie)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_http_cache_fresh_hit() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Arc::new(crate::cache::HttpCache::open(temp.path()).unwrap());

        let url = RixUrl::parse("https://example.com/app.js").unwrap();
        let body = b"console.log('cached version');".to_vec();

        let mut headers = HashMap::new();
        headers.insert("cache-control".to_string(), "public, max-age=86400".to_string());
        headers.insert("etag".to_string(), "\"app-v1\"".to_string());
        headers.insert("content-type".to_string(), "application/javascript".to_string());

        let seeded_response = HttpResponse {
            status: 200,
            headers,
            body: body.clone(),
            final_url: url.clone(),
            content_type: "application/javascript".to_string(),
            protocol: NegotiatedProtocol::H2,
            navigation_timing: NavigationTiming::default(),
            navigation_started: Instant::now(),
            set_cookies: Vec::new(),
        };

        cache.put(url.as_str(), &seeded_response).unwrap();

        let client = HttpClient::new(None).unwrap().with_http_cache(Some(cache));

        let res = client.fetch(&url).await.expect("Fresh cache hit must succeed without network");
        assert_eq!(res.status, 200);
        assert_eq!(res.body, body);
        assert_eq!(res.headers.get("etag").unwrap(), "\"app-v1\"");
        assert!(res.headers.contains_key("age"));
    }

    #[tokio::test]
    async fn test_client_http_cache_bypass_on_no_store() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Arc::new(crate::cache::HttpCache::open(temp.path()).unwrap());

        let url = RixUrl::parse("http://127.0.0.1:9/app.js").unwrap();
        let body = b"console.log('cached version');".to_vec();

        let mut headers = HashMap::new();
        headers.insert("cache-control".to_string(), "public, max-age=86400".to_string());
        let seeded_response = HttpResponse {
            status: 200,
            headers,
            body: body.clone(),
            final_url: url.clone(),
            content_type: "application/javascript".to_string(),
            protocol: NegotiatedProtocol::Http11,
            navigation_timing: NavigationTiming::default(),
            navigation_started: Instant::now(),
            set_cookies: Vec::new(),
        };
        cache.put(url.as_str(), &seeded_response).unwrap();

        let normal_client = HttpClient::new(None).unwrap().with_http_cache(Some(cache.clone()));
        let cached_res = normal_client.fetch(&url).await.expect("Cache hit must succeed");
        assert_eq!(cached_res.status, 200);
        assert_eq!(cached_res.body, body);

        let mut no_store_client = HttpClient::new(None).unwrap().with_http_cache(Some(cache));
        no_store_client.request_headers.insert(
            hyper::header::CACHE_CONTROL,
            hyper::header::HeaderValue::from_static("no-store"),
        );

        let res = no_store_client.fetch(&url).await;
        assert!(res.is_err(), "no-store must bypass cache and fail to connect to 127.0.0.1:9");
    }

    #[tokio::test]
    async fn test_client_http_cache_304_revalidation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req_text = String::from_utf8_lossy(&buf[..n]);
                if req_text.contains("if-none-match: \"v1\"") {
                    let now_date = crate::cache::fmt_http_date(crate::cache::now_secs());
                    let response = format!(
                        "HTTP/1.1 304 Not Modified\r\nDate: {now_date}\r\nCache-Control: max-age=3600\r\nETag: \"v1\"\r\n\r\n"
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                } else {
                    let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
                    let _ = socket.write_all(response.as_bytes()).await;
                }
            }
        });

        let temp = tempfile::tempdir().unwrap();
        let cache = Arc::new(crate::cache::HttpCache::open(temp.path()).unwrap());

        let url = RixUrl::parse(&format!("http://127.0.0.1:{port}/script.js")).unwrap();
        let body = b"revalidated cached body content".to_vec();

        let mut headers = HashMap::new();
        headers.insert("cache-control".to_string(), "max-age=0, must-revalidate".to_string());
        headers.insert("etag".to_string(), "\"v1\"".to_string());
        headers.insert("date".to_string(), "Sun, 06 Nov 1994 08:49:37 GMT".to_string());

        let seeded_response = HttpResponse {
            status: 200,
            headers,
            body: body.clone(),
            final_url: url.clone(),
            content_type: "application/javascript".to_string(),
            protocol: NegotiatedProtocol::Http11,
            navigation_timing: NavigationTiming::default(),
            navigation_started: Instant::now(),
            set_cookies: Vec::new(),
        };
        cache.put(url.as_str(), &seeded_response).unwrap();

        let client = HttpClient::new(None).unwrap().with_http_cache(Some(cache.clone()));

        let res = client.fetch(&url).await.expect("Revalidation fetch must succeed");
        assert_eq!(res.status, 200);
        assert_eq!(res.body, body);
        assert!(res.headers.contains_key("date"));
        assert_eq!(res.headers.get("cache-control").unwrap(), "max-age=3600");

        let second_res = client.fetch(&url).await.expect("Subsequent fetch should be fresh hit");
        assert_eq!(second_res.status, 200);
        assert_eq!(second_res.body, body);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_client_preconnect_and_dns_prefetch() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = socket.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        let response = "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: keep-alive\r\n\r\npong";
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.flush().await;
                    }
                });
            }
        });

        let client = HttpClient::new(None).unwrap();
        client.dns_prefetch("127.0.0.1").await;

        let target = RixUrl::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        client.preconnect(&target).await;

        let res = client.fetch(&target).await.expect("Fetch via preconnected pool should succeed");
        assert_eq!(res.status, 200);
        assert_eq!(res.body, b"pong");
    }
}

