
#![deny(unsafe_code)]

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info};

#[derive(Debug, Deserialize)]
pub struct NavigateRequest {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ScreenshotRequest {
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct ClickRequest {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Deserialize)]
pub struct TypeRequest {
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct KeyRequest {
    pub key: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub meta: bool,
}

#[derive(Debug, Deserialize)]
pub struct EvalRequest {
    pub script: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct TabInfo {
    pub id: u32,
    pub url: String,
    pub title: String,
}

pub enum ControlMessage {
    Status {
        resp: oneshot::Sender<Result<serde_json::Value, String>>,
    },
    Navigate {
        url: String,
        resp: oneshot::Sender<Result<serde_json::Value, String>>,
    },
    Screenshot {
        path: String,
        resp: oneshot::Sender<Result<serde_json::Value, String>>,
    },
    Click {
        x: f32,
        y: f32,
        resp: oneshot::Sender<Result<serde_json::Value, String>>,
    },
    Type {
        text: String,
        resp: oneshot::Sender<Result<serde_json::Value, String>>,
    },
    Key {
        key: String,
        code: String,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
        resp: oneshot::Sender<Result<serde_json::Value, String>>,
    },
    GetDom {
        resp: oneshot::Sender<Result<serde_json::Value, String>>,
    },
    EvalJs {
        script: String,
        resp: oneshot::Sender<Result<serde_json::Value, String>>,
    },
}

impl ControlMessage {
    pub fn requires_redraw(&self) -> bool {
        match self {
            ControlMessage::GetDom { .. } | ControlMessage::Screenshot { .. } => false,
            _ => true,
        }
    }
}

pub fn start_control_server(
    tx: mpsc::Sender<ControlMessage>,
    port: u16,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let addr = format!("127.0.0.1:{}", port);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => {
                info!(
                    "RixBrowser Remote Control & MCP API listening on http://{}",
                    addr
                );
                l
            }
            Err(e) => {
                error!(
                    "Failed to bind RixBrowser control server to {}: {}",
                    addr, e
                );
                return;
            }
        };

        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => break,
            };

            let tx_clone = tx.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                let mut collected: Vec<u8> = Vec::new();
                let header_end = loop {
                    let n = match socket.read(&mut buf).await {
                        Ok(n) if n > 0 => n,
                        _ => return,
                    };
                    collected.extend_from_slice(&buf[..n]);
                    if let Some(index) = find_header_end(&collected) {
                        break index;
                    }
                    if collected.len() > 1 << 20 {
                        return;
                    }
                };
                let header_text = String::from_utf8_lossy(&collected[..header_end]).to_string();
                let content_length = parse_content_length(&header_text);
                let body_start = header_end
                    + if collected[header_end..].starts_with(b"\r\n\r\n") {
                        4
                    } else {
                        2
                    };
                while collected.len() < body_start + content_length {
                    let n = match socket.read(&mut buf).await {
                        Ok(n) if n > 0 => n,
                        _ => break,
                    };
                    collected.extend_from_slice(&buf[..n]);
                }
                let req_str = String::from_utf8_lossy(&collected).to_string();
                let (status, resp_body) = handle_http_request(&tx_clone, &req_str).await;

                let response = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    resp_body.len(),
                    resp_body
                );

                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    })
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    let crlf = bytes.windows(4).position(|window| window == b"\r\n\r\n");
    let lf = bytes.windows(2).position(|window| window == b"\n\n");
    match (crlf, lf) {
        (Some(crlf), Some(lf)) => Some(crlf.min(lf)),
        (Some(crlf), None) => Some(crlf),
        (None, Some(lf)) => Some(lf),
        (None, None) => None,
    }
}

fn parse_content_length(header_text: &str) -> usize {
    header_text
        .lines()
        .find_map(|line| {
            let mut parts = line.splitn(2, ':');
            let name = parts.next()?.trim();
            if !name.eq_ignore_ascii_case("content-length") {
                return None;
            }
            parts.next()?.trim().parse::<usize>().ok()
        })
        .unwrap_or(0)
}

async fn handle_http_request(
    tx: &mpsc::Sender<ControlMessage>,
    req_str: &str,
) -> (&'static str, String) {
    let mut lines = req_str.lines();
    let request_line = match lines.next() {
        Some(l) => l,
        None => {
            return (
                "400 Bad Request",
                json!({"error": "Empty request"}).to_string(),
            )
        }
    };

    let has_origin_header = req_str
        .lines()
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .any(|line| {
            line.split_once(':')
                .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case("origin"))
        });
    if has_origin_header {
        return (
            "403 Forbidden",
            json!({"error": "Requests with an Origin header are not allowed"}).to_string(),
        );
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");

    if method == "OPTIONS" {
        return ("200 OK", json!({"ok": true}).to_string());
    }

    let body = if let Some(idx) = req_str.find("\r\n\r\n") {
        &req_str[idx + 4..]
    } else if let Some(idx) = req_str.find("\n\n") {
        &req_str[idx + 2..]
    } else {
        ""
    };

    match (method, path) {
        ("GET", "/health") | ("GET", "/status") => {
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(ControlMessage::Status { resp: resp_tx })
                .await
                .is_err()
            {
                return (
                    "503 Service Unavailable",
                    json!({"error": "Browser shell unavailable"}).to_string(),
                );
            }
            match resp_rx.await {
                Ok(Ok(val)) => ("200 OK", val.to_string()),
                Ok(Err(e)) => ("500 Internal Error", json!({"error": e}).to_string()),
                Err(_) => (
                    "500 Internal Error",
                    json!({"error": "Channel dropped"}).to_string(),
                ),
            }
        }

        ("POST", "/navigate") => {
            let nav: NavigateRequest = match serde_json::from_str(body) {
                Ok(r) => r,
                Err(e) => {
                    return (
                        "400 Bad Request",
                        json!({"error": format!("Invalid JSON: {e}")}).to_string(),
                    )
                }
            };

            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(ControlMessage::Navigate {
                    url: nav.url,
                    resp: resp_tx,
                })
                .await
                .is_err()
            {
                return (
                    "503 Service Unavailable",
                    json!({"error": "Browser shell unavailable"}).to_string(),
                );
            }
            match resp_rx.await {
                Ok(Ok(val)) => ("200 OK", val.to_string()),
                Ok(Err(e)) => ("500 Internal Error", json!({"error": e}).to_string()),
                Err(_) => (
                    "500 Internal Error",
                    json!({"error": "Channel dropped"}).to_string(),
                ),
            }
        }

        ("POST", "/screenshot") => {
            let req: ScreenshotRequest = serde_json::from_str(body).unwrap_or(ScreenshotRequest {
                path: String::new(),
            });

            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(ControlMessage::Screenshot {
                    path: req.path,
                    resp: resp_tx,
                })
                .await
                .is_err()
            {
                return (
                    "503 Service Unavailable",
                    json!({"error": "Browser shell unavailable"}).to_string(),
                );
            }
            match resp_rx.await {
                Ok(Ok(val)) => ("200 OK", val.to_string()),
                Ok(Err(e)) => ("500 Internal Error", json!({"error": e}).to_string()),
                Err(_) => (
                    "500 Internal Error",
                    json!({"error": "Channel dropped"}).to_string(),
                ),
            }
        }

        ("POST", "/click") => {
            let req: ClickRequest = match serde_json::from_str(body) {
                Ok(r) => r,
                Err(e) => {
                    return (
                        "400 Bad Request",
                        json!({"error": format!("Invalid JSON: {e}")}).to_string(),
                    )
                }
            };

            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(ControlMessage::Click {
                    x: req.x,
                    y: req.y,
                    resp: resp_tx,
                })
                .await
                .is_err()
            {
                return (
                    "503 Service Unavailable",
                    json!({"error": "Browser shell unavailable"}).to_string(),
                );
            }
            match resp_rx.await {
                Ok(Ok(val)) => ("200 OK", val.to_string()),
                Ok(Err(e)) => ("500 Internal Error", json!({"error": e}).to_string()),
                Err(_) => (
                    "500 Internal Error",
                    json!({"error": "Channel dropped"}).to_string(),
                ),
            }
        }

        ("POST", "/type") => {
            let req: TypeRequest = match serde_json::from_str(body) {
                Ok(r) => r,
                Err(e) => {
                    return (
                        "400 Bad Request",
                        json!({"error": format!("Invalid JSON: {e}")}).to_string(),
                    )
                }
            };

            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(ControlMessage::Type {
                    text: req.text,
                    resp: resp_tx,
                })
                .await
                .is_err()
            {
                return (
                    "503 Service Unavailable",
                    json!({"error": "Browser shell unavailable"}).to_string(),
                );
            }
            match resp_rx.await {
                Ok(Ok(val)) => ("200 OK", val.to_string()),
                Ok(Err(e)) => ("500 Internal Error", json!({"error": e}).to_string()),
                Err(_) => (
                    "500 Internal Error",
                    json!({"error": "Channel dropped"}).to_string(),
                ),
            }
        }

        ("POST", "/key") => {
            let req: KeyRequest = match serde_json::from_str(body) {
                Ok(r) => r,
                Err(e) => {
                    return (
                        "400 Bad Request",
                        json!({"error": format!("Invalid JSON: {e}")}).to_string(),
                    )
                }
            };

            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(ControlMessage::Key {
                    key: req.key,
                    code: req.code,
                    ctrl: req.ctrl,
                    alt: req.alt,
                    shift: req.shift,
                    meta: req.meta,
                    resp: resp_tx,
                })
                .await
                .is_err()
            {
                return (
                    "503 Service Unavailable",
                    json!({"error": "Browser shell unavailable"}).to_string(),
                );
            }
            match resp_rx.await {
                Ok(Ok(val)) => ("200 OK", val.to_string()),
                Ok(Err(e)) => ("500 Internal Error", json!({"error": e}).to_string()),
                Err(_) => (
                    "500 Internal Error",
                    json!({"error": "Channel dropped"}).to_string(),
                ),
            }
        }

        ("GET", "/dom") => {
            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(ControlMessage::GetDom { resp: resp_tx })
                .await
                .is_err()
            {
                return (
                    "503 Service Unavailable",
                    json!({"error": "Browser shell unavailable"}).to_string(),
                );
            }
            match resp_rx.await {
                Ok(Ok(val)) => ("200 OK", val.to_string()),
                Ok(Err(e)) => ("500 Internal Error", json!({"error": e}).to_string()),
                Err(_) => (
                    "500 Internal Error",
                    json!({"error": "Channel dropped"}).to_string(),
                ),
            }
        }

        ("POST", "/eval") => {
            let req: EvalRequest = match serde_json::from_str(body) {
                Ok(r) => r,
                Err(e) => {
                    return (
                        "400 Bad Request",
                        json!({"error": format!("Invalid JSON: {e}")}).to_string(),
                    )
                }
            };

            let (resp_tx, resp_rx) = oneshot::channel();
            if tx
                .send(ControlMessage::EvalJs {
                    script: req.script,
                    resp: resp_tx,
                })
                .await
                .is_err()
            {
                return (
                    "503 Service Unavailable",
                    json!({"error": "Browser shell unavailable"}).to_string(),
                );
            }
            match resp_rx.await {
                Ok(Ok(val)) => ("200 OK", val.to_string()),
                Ok(Err(e)) => ("500 Internal Error", json!({"error": e}).to_string()),
                Err(_) => (
                    "500 Internal Error",
                    json!({"error": "Channel dropped"}).to_string(),
                ),
            }
        }

        _ => (
            "404 Not Found",
            json!({"error": "Unknown endpoint"}).to_string(),
        ),
    }
}
