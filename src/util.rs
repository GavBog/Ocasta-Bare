use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, Response},
};
use memory_stats::memory_stats;
use serde_json::{json, to_string_pretty};
#[cfg(feature = "v2")]
use std::{collections::HashMap, sync::Arc};
#[cfg(feature = "v2")]
use tokio::sync::{mpsc::Receiver, Mutex};

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
    let mut versions = vec![];

    if !cfg!(feature = "proxy") {
        versions.push("None");
    }

    #[cfg(feature = "v1")]
    versions.push("v1");
    #[cfg(feature = "v2")]
    versions.push("v2");
    #[cfg(feature = "v3")]
    versions.push("v3");

    let json = json!(
        {
            "versions": versions,
            "language": "Rust",
            "memoryUsage": f64::from((memory_stats().unwrap().physical_mem as f64 / 1024.0 / 1024.0) * 100.0).round() / 100.0,
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

pub fn join_headers(headers: HeaderMap) -> Result<HeaderValue, ()> {
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

            let id: usize = if let Ok(id) = key.as_str().replace("x-bare-headers-", "").parse() {
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

#[cfg(feature = "v2")]
pub async fn db_manager(
    db: Arc<Mutex<HashMap<String, String>>>,
    mut rx: Receiver<(String, String)>,
) {
    loop {
        let (key, value) = rx.recv().await.unwrap();
        let key1 = key.clone();
        let mut db1 = db.lock().await;
        let db3 = db.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(30000)).await;
            let mut db = db3.lock().await;
            db.remove(&key1);
        });

        db1.insert(key, value);
    }
}
