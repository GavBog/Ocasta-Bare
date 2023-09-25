use argh::{from_env, FromArgs};
use axum::{
    routing::{any, get},
    Router,
};
use ocastabare::{util::index, v3};
use std::net::SocketAddr;

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

    let app = Router::new().route(&init.directory, get(index)).route(
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
