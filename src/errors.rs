use axum::{
    body::Body,
    http::{Response, StatusCode},
};

pub fn error_response(status: StatusCode) -> Response<Body> {
    let mut res = Response::default();
    *res.status_mut() = status;
    *res.body_mut() = Body::from(format!("Error: {}", status));
    res
}
