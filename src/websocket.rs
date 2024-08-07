use axum::{
    body::Body,
    extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
    http::{HeaderMap, Request},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{http, Message as TungsteniteMessage},
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

    let url = msg["remote"].as_str().unwrap_or_default();
    let headers = match &msg["headers"] {
        Value::Object(headers) => headers,
        _ => return,
    };

    let mut new_headers = http::header::HeaderMap::new();
    for (key, value) in headers {
        let key = if let Ok(key) = http::header::HeaderName::from_bytes(key.as_bytes()) {
            key
        } else {
            continue;
        };
        let value = if let Ok(value) =
            http::header::HeaderValue::from_str(value.as_str().unwrap_or_default())
        {
            value
        } else {
            continue;
        };
        new_headers.insert(key, value);
    }

    let forward_headers = match &msg["forwardHeaders"] {
        Value::Array(headers) => headers,
        _ => return,
    };

    for key in forward_headers {
        let key: http::header::HeaderName =
            if let Ok(key) = key.as_str().unwrap_or_default().parse() {
                key
            } else {
                continue;
            };
        if let Some(value) = req_headers.get(key.as_str()) {
            let value: http::header::HeaderValue =
                if let Ok(value) = value.to_str().unwrap_or_default().parse() {
                    value
                } else {
                    continue;
                };
            new_headers.insert(key, value.clone());
        }
    }

    let mut server = http::Request::builder()
        .uri(url)
        .body(())
        .unwrap_or_default();
    *server.headers_mut() = new_headers;

    let (mut socket, res) = match connect_async(server).await {
        Ok((socket, res)) => (socket, res),
        _ => return,
    };

    let protocol = res
        .headers()
        .get(http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    let set_cookies: Vec<String> = res
        .headers()
        .get_all(http::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .collect();

    let msg = json!({
        "type": "open",
        "protocol": protocol,
        "setCookies": set_cookies,
    });

    let _ = session.send(AxumMessage::Text(msg.to_string())).await;

    loop {
        tokio::select! {
            Some(Ok(msg)) = session.next() => {
                let msg = axum_message_handler(msg.clone());
                if msg == TungsteniteMessage::Close(None) {
                    let _ = socket.send(msg).await;
                    break;
                }
                if socket.send(msg).await.is_err() {
                    break;
                }
            },
            Some(Ok(msg)) = socket.next() => {
                let msg = tungstenite_message_handler(msg.clone());
                if msg == AxumMessage::Close(None) {
                    let _ = session.send(msg).await;
                    break;
                }
                if session.send(msg).await.is_err() {
                    break;
                }
            },
            else => break,
        }
    }

    let _ = socket.close(None).await;
    let _ = session.close().await;
}

fn axum_message_handler(msg: AxumMessage) -> TungsteniteMessage {
    match msg {
        AxumMessage::Text(text) => TungsteniteMessage::Text(text),
        AxumMessage::Binary(bin) => TungsteniteMessage::Binary(bin),
        AxumMessage::Ping(msg) => TungsteniteMessage::Ping(msg),
        AxumMessage::Pong(msg) => TungsteniteMessage::Pong(msg),
        AxumMessage::Close(_) => TungsteniteMessage::Close(None),
    }
}

fn tungstenite_message_handler(msg: TungsteniteMessage) -> AxumMessage {
    match msg {
        TungsteniteMessage::Text(text) => AxumMessage::Text(text),
        TungsteniteMessage::Binary(bin) => AxumMessage::Binary(bin),
        TungsteniteMessage::Ping(msg) => AxumMessage::Ping(msg),
        TungsteniteMessage::Pong(msg) => AxumMessage::Pong(msg),
        TungsteniteMessage::Close(_) => AxumMessage::Close(None),
        _ => AxumMessage::Close(None),
    }
}
