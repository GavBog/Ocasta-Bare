use axum::{
    routing::{any, get},
    Router,
};
use ocastabare::{o3, util::index, v3};
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(index))
        .route("/v3/", any(v3::proxy))
        .route("/o3/", get(o3::proxy));

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
