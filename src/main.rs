use axum::{
    body::{Body, Bytes},
    extract,
    http::{HeaderMap, HeaderValue, Request, Response, StatusCode},
    routing::{any, get, post},
    Router,
};
use ocastaproxy::{
    codec::{decode, encode},
    errors, websocket,
};
use serde::Deserialize;
use std::{collections::HashMap, net::SocketAddr};

#[derive(Deserialize)]
struct FormData {
    url: String,
}

async fn gateway(extract::Path(path): extract::Path<String>, body: Bytes) -> Response<Body> {
    let mut url = if let Ok(data) = serde_urlencoded::from_bytes::<FormData>(&body) {
        data.url
    } else {
        return errors::error_response(StatusCode::BAD_REQUEST);
    };
    if !url.starts_with("http") {
        url = format!("https://{}", url);
    }

    let encoding = path.as_str();
    url = encode(url, encoding.to_string());
    url = format!("/{}/{}", encoding, url);

    let header = if let Ok(header) = HeaderValue::from_str(&url) {
        header
    } else {
        return errors::error_response(StatusCode::BAD_REQUEST);
    };

    let mut headers = HeaderMap::new();
    headers.insert("location", header);

    let mut res = Response::default();
    *res.status_mut() = StatusCode::SEE_OTHER;
    *res.headers_mut() = headers;

    res
}

async fn proxy(
    extract::Path((encoding, url)): extract::Path<(String, String)>,
    extract::Query(query): extract::Query<HashMap<String, String>>,
    headers: HeaderMap,
    req: Request<Body>,
) -> Response<Body> {
    let mut url = if let Ok(url) = reqwest::Url::parse(&decode(url, encoding.clone())) {
        url
    } else {
        return errors::error_response(StatusCode::BAD_REQUEST);
    };

    let query = query
        .iter()
        .map(|(key, value)| {
            if value.is_empty() {
                key.clone()
            } else {
                format!("{}={}", key, value)
            }
        })
        .collect::<Vec<String>>()
        .join("&");

    if !query.is_empty() {
        url.set_query(Some(&query));
    }

    let origin = url.origin().ascii_serialization();
    let mut new_headers = HeaderMap::new();
    for (key, value) in headers.iter() {
        match key.as_str() {
            "host"
            | "accept-encoding"
            | "forwarded"
            | "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
            | "x-real-ip"
            | "x-envoy-external-address" => {}
            "origin" => {
                if let Ok(header_value) = HeaderValue::from_str(&origin) {
                    new_headers.insert(key.clone(), header_value);
                }
            }
            "referer" => {
                if let Ok(header_value) = HeaderValue::from_str(&origin) {
                    new_headers.insert(key.clone(), header_value);
                }
            }
            _ => {
                new_headers.insert(key.clone(), value.clone());
            }
        }
    }

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
        _ => return errors::error_response(StatusCode::BAD_REQUEST),
    };

    let request = request_builder
        .headers(new_headers)
        .body(req.into_body())
        .build();

    let request = if let Ok(request) = request {
        request
    } else {
        return errors::error_response(StatusCode::BAD_REQUEST);
    };

    let response = client.execute(request).await;

    let response = if let Ok(response) = response {
        response
    } else {
        return errors::error_response(StatusCode::BAD_REQUEST);
    };

    let status = response.status();
    let mut response_headers = response.headers().clone();
    response_headers.remove("content-security-policy");
    response_headers.remove("content-security-policy-report-only");
    response_headers.remove("strict-transport-security");
    response_headers.remove("x-content-type-options");
    response_headers.remove("x-frame-options");
    let content_type = if let Some(content_type) = response_headers.get("content-type") {
        content_type
    } else {
        return errors::error_response(StatusCode::BAD_REQUEST);
    };

    if content_type.to_str().unwrap_or("").starts_with("image/") {
        let mut res = Response::default();
        *res.status_mut() = status;
        *res.headers_mut() = response_headers;
        *res.body_mut() = response.bytes().await.unwrap_or_default().into();
        return res;
    }

    let page = if let Ok(page) = response.text().await {
        page
    } else {
        return errors::error_response(StatusCode::BAD_REQUEST);
    };

    response_headers.insert(
        "content-length",
        if let Ok(content_length) = HeaderValue::from_str(&page.len().to_string()) {
            content_length
        } else {
            return errors::error_response(StatusCode::BAD_REQUEST);
        },
    );

    let mut res = Response::default();
    *res.status_mut() = status;
    *res.headers_mut() = response_headers;
    *res.body_mut() = page.into();
    res
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/:encoding/gateway", post(gateway))
        .route("/:encoding/*url", any(proxy))
        .route("/ws/:encoding/*url", get(websocket::proxy));

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
