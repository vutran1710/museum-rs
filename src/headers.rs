//! HTTP headers a store sends with every request, shared by all built-in HTTP stores (GitHub
//! today). The types are the `http` crate's, the same ones every Rust HTTP client uses.

pub use http::header::ACCEPT;
pub use http::header::AUTHORIZATION;
pub use http::header::HeaderMap;
pub use http::header::HeaderName;
pub use http::header::HeaderValue;
pub use http::header::InvalidHeaderValue;
pub use http::header::USER_AGENT;

/// `Authorization: Bearer <token>`, ready for a store's `headers(..)`.
pub fn bearer(token: &str) -> Result<HeaderMap, InvalidHeaderValue> {
    Ok(HeaderMap::from_iter([(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}"))?,
    )]))
}
