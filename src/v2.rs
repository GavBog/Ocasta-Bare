use crate::{
    util::{index, join_headers, split_headers},
    websocket,
};
use axum::{
    body::{Body, Bytes},
    extract::{Query, WebSocketUpgrade},
    http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode},
    response::IntoResponse,
};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc::Sender, Mutex};

pub async fn proxy(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    ws: Option<WebSocketUpgrade>,
    req: Request<Body>,
    map: Arc<Mutex<HashMap<String, String>>>,
) -> impl IntoResponse {
    if let Some(ws) = ws {
        return websocket::v2(ws, req, map).await.into_response();
    }

    let cache = query.contains_key("cache");
    let mut base_forward_headers = vec![
        "accept-encoding",
        "accept-language",
        "sec-websocket-extensions",
        "sec-websocket-key",
        "sec-websocket-version",
    ];
    let mut base_pass_headers = vec!["content-encoding", "content-length", "last-modified"];
    let mut base_pass_status = vec![];

    if cache {
        base_forward_headers.extend_from_slice(&[
            "if-modified-since",
            "if-none-match",
            "cache-control",
        ]);
        base_pass_headers.extend_from_slice(&["cache-control", "etag"]);
        base_pass_status.extend_from_slice(&["304"]);
    }

    let protocol = if let Some(protocol) = headers
        .get("X-Bare-Protocol")
        .and_then(|value| value.to_str().ok())
    {
        protocol
    } else {
        return index().await.into_response();
    };
    let host = if let Some(host) = headers
        .get("X-Bare-Host")
        .and_then(|value| value.to_str().ok())
    {
        host
    } else {
        if let Some(host) = headers.get("host").and_then(|value| value.to_str().ok()) {
            host
        } else {
            return index().await.into_response();
        }
    };
    let port = if let Some(port) = headers
        .get("X-Bare-Port")
        .and_then(|value| value.to_str().ok())
    {
        port
    } else {
        return index().await.into_response();
    };
    let path = if let Some(path) = headers
        .get("X-Bare-Path")
        .and_then(|value| value.to_str().ok())
    {
        path
    } else {
        return index().await.into_response();
    };

    let url = format!("{}//{}:{}{}", protocol, host, port, path);

    let bare_headers = if let Ok(bare_headers) = join_headers(headers.clone()) {
        bare_headers
    } else {
        HeaderValue::from_static("[]")
    };

    let bare_headers =
        if let Ok(bare_headers) = serde_json::from_str(bare_headers.to_str().unwrap_or_default()) {
            bare_headers
        } else {
            Value::Object(serde_json::Map::new())
        };

    let bare_headers = if let Value::Object(bare_headers) = bare_headers {
        bare_headers
    } else {
        serde_json::Map::new()
    };

    let mut new_headers = HeaderMap::new();
    for (key, value) in bare_headers {
        if let Ok(key) = HeaderName::from_bytes(key.as_bytes()) {
            if let Ok(value) = HeaderValue::from_str(value.as_str().unwrap_or_default()) {
                new_headers.insert(key, value);
            }
        }
    }

    headers
        .get("X-Bare-Forward-Headers")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(",")
        .filter(|key| match *key {
            "connection" | "transfer-encoding" | "host" | "origin" | "referer" => false,
            _ => true,
        })
        .chain(base_forward_headers)
        .for_each(|key| {
            if let Some(value) = headers.get(key) {
                if let Ok(key) = HeaderName::from_bytes(key.as_bytes()) {
                    new_headers.insert(key, value.clone());
                }
            }
        });

    new_headers.remove("host");

    let client = reqwest::Client::new();
    let request_builder = client
        .request(req.method().clone(), url)
        .headers(new_headers)
        .body(req.into_body());

    let response = if let Ok(response) = request_builder.send().await {
        response
    } else {
        let mut res = Response::default();
        *res.status_mut() = StatusCode::BAD_REQUEST;
        *res.body_mut() = Body::from("Bad Request");

        return res.into_response();
    };

    let status = response.status();

    let response_headers = response.headers().clone();
    let mut new_headers = HeaderMap::new();

    headers
        .get("X-Bare-Pass-Headers")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(",")
        .filter(|key| match *key {
            "vary"
            | "connection"
            | "transfer-encoding"
            | "access-control-allow-headers"
            | "access-control-allow-methods"
            | "access-control-expose-headers"
            | "access-control-max-age"
            | "access-control-request-headers"
            | "access-control-request-method" => false,
            _ => true,
        })
        .chain(base_pass_headers)
        .for_each(|key| {
            if let Some(value) = response_headers.get(key) {
                if let Ok(key) = HeaderName::from_bytes(key.as_bytes()) {
                    new_headers.insert(key, value.clone());
                }
            }
        });

    let mut response_headers_bare: HashMap<String, String> = HashMap::new();
    for key in response_headers.keys() {
        if let Some(value) = response_headers.get(key.as_str()) {
            response_headers_bare.insert(
                key.as_str().to_string(),
                value.to_str().unwrap_or_default().to_string(),
            );
        }
    }

    let response_headers_bare = json!(response_headers_bare).to_string();

    new_headers.insert(
        "x-bare-headers",
        if let Ok(response_headers_bare) = HeaderValue::from_str(&response_headers_bare) {
            response_headers_bare
        } else {
            HeaderValue::from_static("[]")
        },
    );
    new_headers.insert(
        "x-bare-status",
        if let Ok(status) = status.as_str().parse() {
            status
        } else {
            HeaderValue::from_static("200")
        },
    );
    new_headers.insert(
        "x-bare-status-text",
        if let Some(status) = status.canonical_reason() {
            if let Ok(status) = status.parse() {
                status
            } else {
                HeaderValue::from_static("OK")
            }
        } else {
            HeaderValue::from_static("OK")
        },
    );

    for key in [
        "access-control-allow-origin",
        "access-control-allow-headers",
        "access-control-allow-methods",
        "access-control-expose-headers",
    ] {
        new_headers.insert(key, HeaderValue::from_static("*"));
    }

    let page = if let Ok(page) = response.bytes().await {
        page
    } else {
        Bytes::new()
    };

    let bare_pass_status: Vec<&str> = headers
        .get("X-Bare-Pass-Status")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(",")
        .chain(base_pass_status)
        .collect();

    let mut res = Response::default();
    *res.headers_mut() = split_headers(new_headers);
    *res.body_mut() = Body::from(page);

    if bare_pass_status.contains(&status.as_str()) {
        *res.status_mut() = status;
    }

    res.into_response()
}

pub async fn ws_new_meta(headers: HeaderMap, tx: Sender<(String, String)>) -> impl IntoResponse {
    let bare_headers = if let Ok(bare_headers) = join_headers(headers.clone()) {
        bare_headers
    } else {
        HeaderValue::from_static("[]")
    };

    let bare_headers =
        if let Ok(bare_headers) = serde_json::from_str(bare_headers.to_str().unwrap_or_default()) {
            bare_headers
        } else {
            Value::Object(serde_json::Map::new())
        };

    let bare_headers = if let Value::Object(bare_headers) = bare_headers {
        bare_headers
    } else {
        serde_json::Map::new()
    };

    let mut forward_headers = HeaderMap::new();
    let base_forward_headers = vec![
        "accept-encoding",
        "accept-language",
        "sec-websocket-extensions",
        "sec-websocket-key",
        "sec-websocket-version",
    ];

    headers
        .get("X-Bare-Forward-Headers")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(",")
        .filter(|key| match *key {
            "connection" | "transfer-encoding" | "host" | "origin" | "referer" => false,
            _ => true,
        })
        .chain(base_forward_headers)
        .for_each(|key| {
            if let Some(value) = headers.get(key) {
                if let Ok(key) = HeaderName::from_bytes(key.as_bytes()) {
                    forward_headers.insert(key, value.clone());
                }
            }
        });

    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).unwrap();
    let id = hex::encode(bytes);

    let protocol = if let Some(protocol) = headers
        .get("X-Bare-Protocol")
        .and_then(|value| value.to_str().ok())
    {
        protocol
    } else {
        return index().await.into_response();
    };
    let host = if let Some(host) = headers
        .get("X-Bare-Host")
        .and_then(|value| value.to_str().ok())
    {
        host
    } else {
        if let Some(host) = headers.get("host").and_then(|value| value.to_str().ok()) {
            host
        } else {
            return index().await.into_response();
        }
    };
    let port = if let Some(port) = headers
        .get("X-Bare-Port")
        .and_then(|value| value.to_str().ok())
    {
        port
    } else {
        return index().await.into_response();
    };
    let path = if let Some(path) = headers
        .get("X-Bare-Path")
        .and_then(|value| value.to_str().ok())
    {
        path
    } else {
        return index().await.into_response();
    };

    let url = format!("{}//{}:{}{}", protocol, host, port, path);

    let data = json!({
        "expires": chrono::Utc::now().timestamp_millis() + 30000,
        "value": {
            "v": "2",
            "remote": url,
            "sendHeaders": bare_headers,
            "forwardHeaders": forward_headers.iter().map(|(key, value)| (key.as_str(), value.to_str().unwrap_or_default())).collect::<Vec<(&str, &str)>>(),
        },
    });

    tx.send((id, data.to_string())).await.unwrap();
    let mut res = Response::default();
    *res.body_mut() = Body::empty();

    res.into_response()
}

pub async fn ws_meta(
    headers: HeaderMap,
    map: Arc<Mutex<HashMap<String, String>>>,
) -> impl IntoResponse {
    let id = if let Some(id) = headers
        .get("X-Bare-ID")
        .and_then(|value| value.to_str().ok())
    {
        id
    } else {
        return index().await.into_response();
    };
    let mut map = map.lock().await;
    let data = if let Some(data) = map.remove(id) {
        data
    } else {
        return index().await.into_response();
    };

    let mut res = Response::default();
    *res.body_mut() = Body::from(data);

    res.into_response()
}
