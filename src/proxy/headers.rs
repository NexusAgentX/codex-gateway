use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue},
    response::{IntoResponse, Response},
};

use crate::request_id_header;

use super::attempt::UnaryResponse;

pub(super) fn unary_response(
    response: UnaryResponse,
    request_id: &str,
    expose_debug_headers: bool,
    route_strategy: &str,
    upstream_id: &str,
) -> Response {
    let mut response =
        (response.status, response.headers, Body::from(response.body)).into_response();
    set_request_id(response.headers_mut(), request_id);
    set_debug_headers(
        response.headers_mut(),
        expose_debug_headers,
        route_strategy,
        upstream_id,
    );
    response
}

pub(super) fn set_request_id(headers: &mut HeaderMap, request_id: &str) {
    if let Ok(value) = HeaderValue::from_str(request_id) {
        headers.insert(request_id_header(), value);
    }
}

pub(super) fn set_debug_headers(
    headers: &mut HeaderMap,
    expose_debug_headers: bool,
    route_strategy: &str,
    upstream_id: &str,
) {
    if !expose_debug_headers {
        return;
    }
    if let Ok(value) = HeaderValue::from_str(route_strategy) {
        headers.insert(
            HeaderName::from_static("x-codex-gateway-route-strategy"),
            value,
        );
    }
    if let Ok(value) = HeaderValue::from_str(upstream_id) {
        headers.insert(
            HeaderName::from_static("x-codex-gateway-upstream-id"),
            value,
        );
    }
}
