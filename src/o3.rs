// Custom spec for OcastaProxy
// Allows for end to end encryption when connecting to https sites
// No more MITM attacks!

use axum::{
    body::Body,
    extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
    http::{HeaderMap, HeaderName, HeaderValue, Request},
    response::IntoResponse,
};
use futures_util::StreamExt;
use serde_json::Value;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::Mutex,
};

pub async fn proxy(ws: WebSocketUpgrade, req: Request<Body>) -> impl IntoResponse {
    let headers = req.headers().clone();
    ws.on_upgrade(move |session| handle_socket(session, headers))
}

async fn handle_socket(mut session: WebSocket, req_headers: HeaderMap) {
    let msg = match session.next().await {
        Some(Ok(msg)) => msg,
        _ => return,
    };
    let msg = msg.into_text().unwrap_or_default();
    let msg: Value = match serde_json::from_str(msg.as_str()) {
        Ok(msg) => msg,
        _ => return,
    };

    let host = msg["host"].as_str().unwrap_or_default();
    let mut headers = if let Some(headers) = msg["headers"].as_object() {
        headers
            .iter()
            .map(|(key, value)| {
                let key =
                    HeaderName::from_bytes(key.as_bytes()).unwrap_or(HeaderName::from_static(""));
                let value = HeaderValue::from_str(value.as_str().unwrap_or_default())
                    .unwrap_or(HeaderValue::from_static(""));
                (key, value)
            })
            .collect()
    } else {
        HeaderMap::new()
    };

    let forward_headers = msg["forwardHeaders"].as_array().unwrap();
    for key in forward_headers {
        let key: HeaderName = if let Ok(key) = key.as_str().unwrap_or_default().parse() {
            key
        } else {
            continue;
        };
        if let Some(value) = req_headers.get(&key) {
            headers.insert(key, value.clone());
        }
    }

    let sock = if let Ok(sock) = TcpStream::connect(host).await {
        sock
    } else {
        return;
    };

    let (mut sock_read, mut sock_write) = tokio::io::split(sock);
    let session = Arc::new(Mutex::new(session));

    let relay_to_socket = async {
        loop {
            let msg = match session.lock().await.next().await {
                Some(Ok(msg)) => msg,
                _ => break,
            };
            let msg = match msg {
                AxumMessage::Binary(msg) => msg,
                AxumMessage::Text(msg) => {
                    if msg == "TLS HANDSHAKE COMPLETE" {
                        break;
                    }
                    continue;
                }
                _ => continue,
            };
            if sock_write.write_all(&msg).await.is_err() {
                break;
            }
        }
    };

    let relay_from_socket = async {
        let mut buf = [0; 1024];
        loop {
            match sock_read.read(&mut buf).await {
                Ok(0) => {
                    break;
                }
                Ok(len) => {
                    if session
                        .lock()
                        .await
                        .send(AxumMessage::Binary(buf[..len].to_vec()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                _ => {}
            }
        }
    };
    tokio::join!(relay_to_socket, relay_from_socket);
}
