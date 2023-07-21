use crate::errors;
use axum::{
    body::Body,
    extract::{
        self,
        ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
    },
    http::{Request, StatusCode},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{http, Message as TungsteniteMessage},
    MaybeTlsStream, WebSocketStream,
};

#[derive(Deserialize)]
pub struct ProxyData {
    #[serde(flatten)]
    query: std::collections::HashMap<String, String>,
}

pub async fn proxy(
    ws: WebSocketUpgrade,
    extract::Path((_encoding, url)): extract::Path<(String, String)>,
    query: extract::Query<ProxyData>,
    req: Request<Body>,
) -> impl IntoResponse {
    let mut url = if let Ok(url) = reqwest::Url::parse(&url) {
        url
    } else {
        return errors::error_response(StatusCode::BAD_REQUEST).into_response();
    };

    url.query_pairs_mut().clear().extend_pairs(
        query
            .query
            .iter()
            .filter(|(key, _)| !key.starts_with("origin=")),
    );

    let default_origin = String::new();
    let origin = query.query.get("origin").unwrap_or(&default_origin);

    let headers = req
        .headers()
        .iter()
        .map(|(k, v)| match k.as_str() {
            "origin" => (
                k.clone(),
                http::HeaderValue::from_str(origin)
                    .unwrap_or_else(|_| http::HeaderValue::from_static("")),
            ),
            "host" => (
                k.clone(),
                url.host_str()
                    .unwrap_or("")
                    .parse()
                    .unwrap_or_else(|_| http::HeaderValue::from_static("")),
            ),
            _ => (k.clone(), v.clone()),
        })
        .collect::<http::HeaderMap>();

    let mut server = http::Request::builder()
        .uri(url.as_str())
        .body(())
        .unwrap_or_default();
    *server.headers_mut() = headers;

    if let Ok((socket, _)) = connect_async(server).await {
        ws.on_upgrade(move |session| handle_socket(session, socket))
    } else {
        errors::error_response(StatusCode::INTERNAL_SERVER_ERROR).into_response()
    }
}

async fn handle_socket(
    mut session: WebSocket,
    mut socket: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
) {
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
