use anyhow::{anyhow, Result};
use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, Response},
};
use memory_stats::memory_stats;
use serde_json::{json, to_string_pretty};

pub async fn index() -> Response<Body> {
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

    let memory =
        ((memory_stats().unwrap().physical_mem as f64 / 1024.0 / 1024.0) * 100.0).round() / 100.0;
    let json = json!(
        {
            "versions": ["v3"],
            "language": "Rust",
            "memoryUsage": memory,
            "maintainer": {
              "email": "you@example.com",
              "website": "https://www.example.com/"
            },
            "project": {
              "name": "Ocasta-Bare",
              "description": "Rust TOMP implementation",
              "email": "hello@projectocasta.org",
              "website": "https://www.projectocasta.org/",
              "repository": "https://github.com/Project-Ocasta/Ocasta-Bare",
              "version": "0.1.0"
            }
          }
    );

    let mut res = Response::default();
    *res.body_mut() = to_string_pretty(&json).unwrap_or_default().into();
    *res.headers_mut() = headers;
    res
}

pub fn split_headers(headers: HeaderMap) -> HeaderMap {
    let mut output = headers.clone();

    if let Some(value) = headers.get("x-bare-headers") {
        if let Ok(value) = value.to_str() {
            if value.len() > 3072 {
                output.remove("x-bare-headers");

                let mut split = 0;

                for i in (0..value.len()).step_by(3072) {
                    let part = if i + 3072 > value.len() {
                        &value[i..value.len()]
                    } else {
                        &value[i..i + 3072]
                    };

                    let id = split;
                    output.insert(
                        if let Ok(key) =
                            HeaderName::from_bytes(format!("x-bare-headers-{}", id).as_bytes())
                        {
                            key
                        } else {
                            continue;
                        },
                        if let Ok(value) = HeaderValue::from_str(format!(";{}", part).as_str()) {
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

pub fn join_headers(headers: HeaderMap) -> Result<HeaderValue> {
    let mut output = headers.clone();

    if headers.contains_key("x-bare-headers-0") {
        let mut join = vec!["".to_string(); headers.len()];
        for (key, value) in headers.iter() {
            if !key.as_str().starts_with("x-bare-headers") || !value.to_str()?.starts_with(';') {
                continue;
            }
            let id = key
                .as_str()
                .trim_start_matches("x-bare-headers-")
                .parse::<usize>()?;
            join[id] = value.to_str()?.trim_start_matches(';').to_string();
            output.remove(key);
        }
        output.insert(
            "x-bare-headers",
            HeaderValue::from_str(join.join("").as_str())?,
        );
    }
    Ok(output
        .remove("x-bare-headers")
        .unwrap_or(HeaderValue::from_static("")))
}
