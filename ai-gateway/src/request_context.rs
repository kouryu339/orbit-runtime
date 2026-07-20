use std::collections::BTreeMap;
use std::future::Future;

use crate::error::ApiError;
use reqwest::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{RequestBuilder, Url};

tokio::task_local! {
    static REQUEST_HEADERS: BTreeMap<String, String>;
    static ALLOW_INSECURE_HEADERS: bool;
}

pub async fn scope_request_headers<F>(
    headers: BTreeMap<String, String>,
    allow_insecure: bool,
    future: F,
) -> F::Output
where
    F: Future,
{
    REQUEST_HEADERS
        .scope(
            headers,
            ALLOW_INSECURE_HEADERS.scope(allow_insecure, future),
        )
        .await
}

pub fn current_request_headers() -> BTreeMap<String, String> {
    REQUEST_HEADERS
        .try_with(Clone::clone)
        .unwrap_or_else(|_| BTreeMap::new())
}

pub fn allow_insecure_request_headers() -> bool {
    ALLOW_INSECURE_HEADERS
        .try_with(|value| *value)
        .unwrap_or(false)
}

pub fn apply_request_headers(
    mut request: RequestBuilder,
    headers: &BTreeMap<String, String>,
) -> RequestBuilder {
    for (name, value) in headers {
        let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) else {
            tracing::warn!(header = %name, "skip invalid runtime LLM request header name");
            continue;
        };
        if header_name == CONTENT_TYPE {
            tracing::warn!(header = %name, "skip runtime override for Content-Type header");
            continue;
        }
        let Ok(header_value) = HeaderValue::from_str(value) else {
            tracing::warn!(header = %name, "skip invalid runtime LLM request header value");
            continue;
        };
        request = request.header(header_name, header_value);
    }
    request
}

pub fn validate_header_transport(
    url: &str,
    headers: &BTreeMap<String, String>,
) -> crate::error::Result<()> {
    if headers.is_empty() {
        return Ok(());
    }

    let parsed = Url::parse(url)
        .map_err(|e| ApiError::LlmFailed(format!("invalid LLM request URL '{}': {e}", url)))?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if is_loopback_url(&parsed) => {
            tracing::warn!(
                url = %url,
                "runtime LLM request headers are being sent over HTTP to a loopback endpoint; only use this for local development"
            );
            Ok(())
        }
        "http" => {
            if allow_insecure_request_headers() {
                tracing::warn!(
                    url = %url,
                    "runtime LLM request headers are being sent over insecure HTTP because allow_insecure_llm_request_headers=true; credential leakage risk is accepted by the host application"
                );
                return Ok(());
            }
            tracing::warn!(
                url = %url,
                "refusing to send runtime LLM request headers over insecure HTTP"
            );
            Err(ApiError::LlmFailed(
                "refusing to send runtime LLM request headers over insecure HTTP; use HTTPS or a loopback endpoint for local development".to_string(),
            ))
        }
        scheme => Err(ApiError::LlmFailed(format!(
            "refusing to send runtime LLM request headers over unsupported URL scheme '{}'",
            scheme
        ))),
    }
}

fn is_loopback_url(url: &Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false),
        None => false,
    }
}
