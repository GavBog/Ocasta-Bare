use argh::{from_env, FromArgs};
#[cfg(feature = "proxy")]
use axum::routing::any;
#[cfg(feature = "v2")]
use axum::{
    body::Body,
    extract::Query,
    extract::WebSocketUpgrade,
    http::{HeaderMap, Request},
};
use axum::{routing::get, Router};
use ocastabare::util::index;
#[cfg(feature = "v2")]
use ocastabare::v2;
#[cfg(feature = "v3")]
use ocastabare::v3;
use std::net::SocketAddr;
#[cfg(feature = "v2")]
use std::{collections::HashMap, sync::Arc};

#[derive(FromArgs)]
/// Bare server init
struct Init {
    /// the bare server directory, defaults to /
    #[argh(option, short = 'd', default = "String::from(\"/\")")]
    directory: String,

    /// the listening host, defaults to 0.0.0.0
    #[argh(option, short = 'h', default = "String::from(\"0.0.0.0\")")]
    host: String,

    /// the port number, defaults to 80
    #[argh(option, short = 'p', default = "80")]
    port: u16,
}

#[tokio::main]
async fn main() {
    let init: Init = from_env();
    let mut addr_tuple = ([0, 0, 0, 0], init.port);

    for (i, num) in init.host.split('.').enumerate() {
        addr_tuple.0[i] = num.parse::<u8>().unwrap();
    }

    let mut app = Router::new();
    app = app.route(&init.directory, get(index));

    #[cfg(feature = "v2")]
    {
        let db = Arc::new(dashmap::DashMap::new());
        let db1 = db.clone();
        let db2 = db.clone();
        app = app
            .route(
                format!(
                    "{}/v2/",
                    init.directory.strip_suffix("/").unwrap_or_default()
                )
                .as_str(),
                any(
                    move |headers: HeaderMap,
                          Query(query): Query<HashMap<String, String>>,
                          ws: Option<WebSocketUpgrade>,
                          req: Request<Body>| {
                        v2::proxy(headers, Query(query), ws, req, db1)
                    },
                ),
            )
            .route(
                format!(
                    "{}/v2/ws-new-meta/",
                    init.directory.strip_suffix("/").unwrap_or_default()
                )
                .as_str(),
                get(move |headers: HeaderMap| v2::ws_new_meta(headers, db2)),
            )
            .route(
                format!(
                    "{}/v2/ws-meta/",
                    init.directory.strip_suffix("/").unwrap_or_default()
                )
                .as_str(),
                get(move |headers: HeaderMap| v2::ws_meta(headers, db)),
            );
    }
    #[cfg(feature = "v3")]
    let app = app.route(
        format!(
            "{}/v3/",
            init.directory.strip_suffix("/").unwrap_or_default()
        )
        .as_str(),
        any(v3::proxy),
    );

    let addr = SocketAddr::from(addr_tuple);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
