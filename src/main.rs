use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode},
    routing::{any, get},
    Router,
};
use ocastaproxy::{errors, websocket};
use std::net::SocketAddr;

fn split_headers(headers: HeaderMap) -> HeaderMap {
    let mut output = headers.clone();

    if let Some(value) = headers.get("x-bare-headers") {
        if let Ok(value) = value.to_str() {
            if value.len() > 3072 {
                output.remove("x-bare-headers");

                let mut split = 0;

                for i in (0..value.len()).step_by(3072) {
                    let part = &value[i..i + 3072];

                    let id = split;
                    output.insert(
                        if let Ok(key) =
                            HeaderName::from_bytes(format!("x-bare-headers-{}", id).as_bytes())
                        {
                            key
                        } else {
                            continue;
                        },
                        if let Ok(value) = HeaderValue::from_str(format!("{};", part).as_str()) {
                            value
                        } else {
                            continue;
                        },
                    );
                    split += 1;
                }
            }
        }
    }

    output
}

fn join_headers(headers: HeaderMap) -> Result<HeaderValue, ()> {
    if headers.contains_key("x-bare-headers") {
        return Ok(headers
            .get("x-bare-headers")
            .unwrap_or(&HeaderValue::from_static("[]"))
            .clone());
    }

    let mut new_headers = HeaderMap::new();
    for (key, value) in headers.iter() {
        if !key.as_str().starts_with("x-bare-headers-") {
            continue;
        }
        new_headers.insert(key, value.clone());
    }

    if new_headers.len() > 0 {
        let mut join = vec![];
        for (key, value) in headers.iter() {
            if !value.to_str().unwrap_or_default().starts_with(';') {
                return Err(());
            }

            let id = if let Ok(id) = key.as_str().replace("x-bare-headers-", "").parse::<usize>() {
                id
            } else {
                return Err(());
            };

            join[id] = value.to_str().unwrap_or_default().replace(";", "");

            new_headers.remove(key);
        }

        let output = if let Ok(output) = HeaderValue::from_str(join.join("").as_str()) {
            output
        } else {
            return Err(());
        };

        return Ok(output);
    } else {
        return Err(());
    }
}

async fn proxy(headers: HeaderMap, req: Request<Body>) -> Response<Body> {
    let url = headers
        .get("X-Bare-URL")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if url.is_empty() {
        return errors::error_response(StatusCode::BAD_REQUEST);
    }

    let bare_headers = if let Ok(bare_headers) = join_headers(headers.clone()) {
        bare_headers
    } else {
        return errors::error_response(StatusCode::BAD_REQUEST);
    };

    let bare_headers: Vec<(&str, &str)> =
        if let Ok(bare_headers) = serde_json::from_str(bare_headers.to_str().unwrap_or_default()) {
            bare_headers
        } else {
            return errors::error_response(StatusCode::BAD_REQUEST);
        };

    let mut new_headers = HeaderMap::new();
    for (key, value) in bare_headers {
        if let Ok(key) = HeaderName::from_bytes(key.as_bytes()) {
            if let Ok(value) = HeaderValue::from_str(&value) {
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
        .chain(vec![
            "accept-encoding",
            "accept-language",
            "sec-websocket-extensions",
            "sec-websocket-key",
            "sec-websocket-version",
        ])
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

    let response_headers = response.headers().clone();
    let response_headers: Vec<(&str, &str)> = response_headers
        .iter()
        .map(|(key, value)| (key.as_str(), value.to_str().unwrap_or_default()))
        .collect();

    let response_headers_bare =
        if let Ok(response_headers_bare) = serde_json::to_string(&response_headers) {
            response_headers_bare
        } else {
            return errors::error_response(StatusCode::BAD_REQUEST);
        };

    let page = if let Ok(page) = response.bytes().await {
        page
    } else {
        return errors::error_response(StatusCode::BAD_REQUEST);
    };

    let mut new_headers = HeaderMap::new();
    new_headers.insert(
        "Content-Length",
        if let Ok(content_length) = page.len().to_string().parse() {
            content_length
        } else {
            return errors::error_response(StatusCode::BAD_REQUEST);
        },
    );
    new_headers.insert(
        "x-bare-status",
        if let Ok(status) = status.as_str().parse() {
            status
        } else {
            return errors::error_response(StatusCode::BAD_REQUEST);
        },
    );
    new_headers.insert(
        "x-bare-status-text",
        if let Some(status) = status.canonical_reason() {
            if let Ok(status) = status.parse() {
                status
            } else {
                return errors::error_response(StatusCode::BAD_REQUEST);
            }
        } else {
            return errors::error_response(StatusCode::BAD_REQUEST);
        },
    );
    new_headers.insert(
        "x-bare-headers",
        if let Ok(response_headers_bare) = HeaderValue::from_str(&response_headers_bare) {
            response_headers_bare
        } else {
            return errors::error_response(StatusCode::BAD_REQUEST);
        },
    );
    let mut new_headers = split_headers(new_headers);

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

    let mut res = Response::default();
    *res.headers_mut() = new_headers;
    *res.body_mut() = page.into();

    if bare_pass_status.contains(&status.as_str()) {
        *res.status_mut() = status;
    }

    res
}

async fn index() -> Response<Body> {
    let mut res = Response::default();
    *res.body_mut() = include_str!("../static/index.json").into();
    res.headers_mut().insert(
        "Content-Type",
        if let Ok(content_type) = HeaderValue::from_str("application/json") {
            content_type
        } else {
            return errors::error_response(StatusCode::BAD_REQUEST);
        },
    );
    res
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(index))
        .route("/v3/ws", get(websocket::proxy))
        .route("/v3", any(proxy));

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
