use axum::http::{HeaderMap, HeaderName, HeaderValue, header};

pub fn forward_request_headers(
    incoming: &HeaderMap,
    upstream_api_key: &str,
) -> Result<HeaderMap, http::header::InvalidHeaderValue> {
    let mut out = HeaderMap::new();
    for (name, value) in incoming {
        if should_forward_request_header(name) {
            out.insert(name.clone(), value.clone());
        }
    }
    out.insert(
        header::AUTHORIZATION,
        authorization_header(upstream_api_key)?,
    );
    out.insert(
        HeaderName::from_static("x-codex-gateway"),
        HeaderValue::from_static("codex-gateway/0.1"),
    );
    Ok(out)
}

pub fn authorization_header(
    upstream_api_key: &str,
) -> Result<HeaderValue, http::header::InvalidHeaderValue> {
    HeaderValue::from_str(&format!("Bearer {upstream_api_key}"))
}

pub fn forward_response_headers(incoming: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in incoming {
        if should_forward_response_header(name) {
            out.insert(name.clone(), value.clone());
        }
    }
    out
}

fn should_forward_request_header(name: &HeaderName) -> bool {
    let name = name.as_str().to_ascii_lowercase();
    if is_hop_by_hop(&name) || is_sensitive_header(&name) {
        return false;
    }
    if matches!(name.as_str(), "host" | "content-length") {
        return false;
    }
    matches!(
        name.as_str(),
        "accept" | "content-type" | "user-agent" | "traceparent" | "tracestate"
    ) || name.starts_with("x-codex-")
        || name.starts_with("x-openai-")
        || name.starts_with("x-responsesapi-")
        || name.starts_with("openai-")
}

fn should_forward_response_header(name: &HeaderName) -> bool {
    let name = name.as_str().to_ascii_lowercase();
    if is_hop_by_hop(&name) || is_sensitive_header(&name) {
        return false;
    }
    !matches!(name.as_str(), "server" | "x-powered-by" | "content-length")
}

pub fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn is_sensitive_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "www-authenticate"
            | "set-cookie"
            | "cookie"
            | "x-api-key"
            | "x-api-key-id"
            | "x-upstream-api-key"
            | "x-openai-api-key"
            | "openai-api-key"
            | "api-key"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sensitive_request_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer cgk_live_a_b"),
        );
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(
            HeaderName::from_static("connection"),
            HeaderValue::from_static("close"),
        );
        headers.insert(
            HeaderName::from_static("x-codex-turn-state"),
            HeaderValue::from_static("state"),
        );
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("secret"),
        );
        headers.insert(
            HeaderName::from_static("cookie"),
            HeaderValue::from_static("secret"),
        );

        let forwarded = forward_request_headers(&headers, "sk-upstream").unwrap();
        assert_eq!(
            forwarded.get(header::AUTHORIZATION).unwrap(),
            "Bearer sk-upstream"
        );
        assert_eq!(forwarded.get(header::ACCEPT).unwrap(), "text/event-stream");
        assert_eq!(forwarded.get("x-codex-turn-state").unwrap(), "state");
        assert!(!forwarded.contains_key("connection"));
        assert!(!forwarded.contains_key("cookie"));
        assert!(!forwarded.contains_key("x-api-key"));
    }

    #[test]
    fn rejects_invalid_upstream_authorization_without_panicking() {
        let headers = HeaderMap::new();
        assert!(forward_request_headers(&headers, "sk-good").is_ok());
        assert!(forward_request_headers(&headers, "sk-bad\r\nx-leak: yes").is_err());
    }

    #[test]
    fn strips_sensitive_response_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(
            HeaderName::from_static("transfer-encoding"),
            HeaderValue::from_static("chunked"),
        );
        headers.insert(
            HeaderName::from_static("server"),
            HeaderValue::from_static("upstream"),
        );
        headers.insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer upstream"),
        );
        headers.insert(header::SET_COOKIE, HeaderValue::from_static("sid=secret"));
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("secret"),
        );

        let forwarded = forward_response_headers(&headers);
        assert_eq!(
            forwarded.get(header::CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );
        assert!(!forwarded.contains_key("transfer-encoding"));
        assert!(!forwarded.contains_key("server"));
        assert!(!forwarded.contains_key(header::WWW_AUTHENTICATE));
        assert!(!forwarded.contains_key(header::SET_COOKIE));
        assert!(!forwarded.contains_key("x-api-key"));
    }
}
