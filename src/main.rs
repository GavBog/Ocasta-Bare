use axum::{
    body::{Body, Bytes},
    extract::WebSocketUpgrade,
    http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode},
    response::IntoResponse,
    routing::{any, get},
    Router,
};
use ocastaproxy::{
    errors,
    util::{join_headers, split_headers},
    websocket,
};
use serde_json::Value;
use std::net::SocketAddr;

async fn proxy(
    headers: HeaderMap,
    ws: Option<WebSocketUpgrade>,
    req: Request<Body>,
) -> impl IntoResponse {
    if let Some(ws) = ws {
        return websocket::proxy(ws, req).await.into_response();
    }

    let url = if let Some(url) = headers
        .get("X-Bare-URL")
        .and_then(|value| value.to_str().ok())
    {
        url
    } else {
        return index().await.into_response();
    };

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
        .chain(vec!["accept-encoding", "accept-language"])
        .for_each(|key| {
            if let Some(value) = headers.get(key) {
                if let Ok(key) = HeaderName::from_bytes(key.as_bytes()) {
                    new_headers.insert(key, value.clone());
                }
            }
        });

    let client = reqwest::Client::new();
    let request_builder = match req.method().as_str() {
        "GET" => client.get(url.clone()),
        "POST" => client.post(url.clone()),
        "PUT" => client.put(url.clone()),
        "DELETE" => client.delete(url.clone()),
        "HEAD" => client.head(url.clone()),
        "OPTIONS" => client.request(reqwest::Method::OPTIONS, url.clone()),
        "CONNECT" => client.request(reqwest::Method::CONNECT, url.clone()),
        "PATCH" => client.patch(url.clone()),
        "TRACE" => client.request(reqwest::Method::TRACE, url.clone()),
        _ => return errors::error_response(StatusCode::BAD_REQUEST).into_response(),
    };

    let request = if let Ok(request) = request_builder
        .headers(new_headers)
        .body(req.into_body())
        .build()
    {
        request
    } else {
        return errors::error_response(StatusCode::BAD_REQUEST).into_response();
    };

    let response = if let Ok(response) = client.execute(request).await {
        response
    } else {
        return errors::error_response(StatusCode::NOT_FOUND).into_response();
    };

    let status = response.status();

    let response_headers = response.headers().clone();
    let mut new_headers = HeaderMap::new();
    if let Some(value) = response_headers.get("Content-Encoding") {
        if let Ok(key) = HeaderName::from_bytes("Content-Encoding".as_bytes()) {
            new_headers.insert(key, value.clone());
        }
    }
    let response_headers: Vec<(&str, &str)> = response_headers
        .iter()
        .map(|(key, value)| (key.as_str(), value.to_str().unwrap_or_default()))
        .collect();

    let response_headers_bare =
        if let Ok(response_headers_bare) = serde_json::to_string(&response_headers) {
            response_headers_bare
        } else {
            "[]".to_string()
        };

    let page = if let Ok(page) = response.bytes().await {
        page
    } else {
        Bytes::new()
    };

    new_headers.insert(
        "Content-Length",
        if let Ok(content_length) = page.len().to_string().parse() {
            content_length
        } else {
            HeaderValue::from_static("0")
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
    new_headers.insert(
        "x-bare-headers",
        if let Ok(response_headers_bare) = HeaderValue::from_str(&response_headers_bare) {
            response_headers_bare
        } else {
            HeaderValue::from_static("[]")
        },
    );

    let bare_pass_headers = headers
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
        .collect::<Vec<&str>>();

    if !bare_pass_headers.is_empty() {
        for (key, value) in response_headers {
            if !bare_pass_headers.contains(&key) {
                continue;
            }

            if let Ok(key) = HeaderName::from_bytes(key.as_bytes()) {
                if let Ok(value) = HeaderValue::from_str(&value) {
                    new_headers.insert(key, value);
                }
            }
        }
    }

    let bare_pass_status = headers
        .get("X-Bare-Pass-Status")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(",")
        .collect::<Vec<&str>>();

    for key in [
        "access-control-allow-origin",
        "access-control-allow-headers",
        "access-control-allow-methods",
        "access-control-expose-headers",
    ] {
        new_headers.insert(key, HeaderValue::from_static("*"));
    }

    let mut res = Response::default();
    *res.headers_mut() = split_headers(new_headers);
    *res.body_mut() = Body::from(page);

    if bare_pass_status.contains(&status.as_str()) {
        *res.status_mut() = status;
    }

    res.into_response()
}

async fn index() -> Response<Body> {
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    for key in [
        "access-control-allow-origin",
        "access-control-allow-headers",
        "access-control-allow-methods",
        "access-control-expose-headers",
    ] {
        headers.insert(key, HeaderValue::from_static("*"));
    }

    let mut res = Response::default();
    *res.body_mut() = include_str!("../static/index.json").into();
    *res.headers_mut() = headers;
    res
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(index))
        .route("/v3/", any(proxy));

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
