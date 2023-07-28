use crate::errors;
use axum::{
    body::Body,
    extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
    http::{HeaderMap, HeaderName, Request, StatusCode},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
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
    let msg: Vec<(String, String)> = match serde_json::from_str(&msg) {
        Ok(msg) => msg,
        _ => return,
    };

    let url = msg
        .iter()
        .find(|(key, _)| key == "remote")
        .map(|(_, value)| value)
        .unwrap();

    let headers = msg
        .iter()
        .find(|(key, _)| key == "headers")
        .map(|(_, value)| value)
        .unwrap();
    let headers: Vec<(String, String)> = match serde_json::from_str(&headers) {
        Ok(headers) => headers,
        _ => return,
    };

    let mut new_headers = HeaderMap::new();
    for (key, value) in headers {
        let key = HeaderName::from_bytes(key.as_bytes()).unwrap();
        new_headers.insert(key, value.parse().unwrap());
    }

    let forward_headers = msg
        .iter()
        .find(|(key, _)| key == "forwardHeaders")
        .map(|(_, value)| value)
        .unwrap();

    let forward_headers: Vec<String> = match serde_json::from_str(&forward_headers) {
        Ok(forward_headers) => forward_headers,
        _ => return,
    };

    for key in forward_headers {
        let key = HeaderName::from_bytes(key.as_bytes()).unwrap();
        if let Some(value) = req_headers.get(&key) {
            new_headers.insert(key, value.clone());
        }
    }

    let mut server = http::Request::builder()
        .uri(url)
        .body(())
        .unwrap_or_default();
    *server.headers_mut() = new_headers;

    let mut socket = match connect_async(server).await {
        Ok((socket, _)) => socket,
        _ => return,
    };

    let msg = r#"{"type":"open","protocol":"","setCookies":[]}"#;
    session
        .send(AxumMessage::Text(msg.to_string()))
        .await
        .unwrap();

    loop {
        tokio::select! {
            Some(Ok(msg)) = session.next() => {
                let msg = axum_message_handler(msg);
                if msg == TungsteniteMessage::Close(None) {
                    let _ = socket.send(msg).await;
                    break;
                }
                if let Err(_) = socket.send(msg).await {
                    break;
                }
            },
            Some(Ok(msg)) = socket.next() => {
                let msg = tungstenite_message_handler(msg);
                if msg == AxumMessage::Close(None) {
                    let _ = session.send(msg).await;
                    break;
                }
                if let Err(_) = session.send(msg).await {
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
