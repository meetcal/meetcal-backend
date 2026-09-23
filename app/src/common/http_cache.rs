//! Strong validators and `Cache-Control` for read-only JSON endpoints.
//!
//! Reference data (meet lists, schedules, records, standards) changes when the
//! scrapers ingest, not per request, so a client that already holds a body can
//! revalidate it with `If-None-Match` and get `304` instead of a re-download.
//! The validator is a SHA-256 of the exact serialized body: a strong ETag, so
//! the same body always yields the same tag across processes and restarts.
use crate::AppError;
use axum::{
    body::Bytes,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// `Cache-Control` for data that changes by ingest, not by request: shared
/// caches may keep it, and a client may reuse it for five minutes before
/// revalidating. Five minutes bounds how stale a schedule can look after a
/// scraper writes; ingest runs on the order of hours.
pub const PUBLIC_MAX_AGE_5_MIN: &str = "public, max-age=300";

/// Strong validator over the exact serialized body.
pub fn strong_etag(body: &[u8]) -> HeaderValue {
    let digest = Sha256::digest(body);
    let mut tag = String::with_capacity(2 + digest.len() * 2);
    tag.push('"');
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(tag, "{byte:02x}");
    }
    tag.push('"');
    HeaderValue::from_str(&tag).expect("hex etag is a valid header value")
}

/// `If-None-Match` matching per RFC 9110 §13.1.2: a `*`, or any listed tag
/// equal to ours after dropping a `W/` weak prefix. The list is bounded by the
/// header size, not by the caller.
pub fn etag_matches(if_none_match: Option<&HeaderValue>, etag: &HeaderValue) -> bool {
    let Some(candidates) = if_none_match.and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let Ok(etag) = etag.to_str() else {
        return false;
    };
    candidates.split(',').map(str::trim).any(|candidate| {
        candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == etag
    })
}

/// A pre-serialized JSON body with its validator: `304` (no body) when the
/// caller's `If-None-Match` names it, otherwise `200` with the body. The
/// `ETag` and, when given, `Cache-Control` go on both answers, as RFC 9110
/// §15.4.5 asks.
pub fn json_response(
    body: Bytes,
    etag: HeaderValue,
    cache_control: Option<&'static str>,
    if_none_match: Option<&HeaderValue>,
) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::ETAG, etag.clone());
    if let Some(cache_control) = cache_control {
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(cache_control),
        );
    }
    if etag_matches(if_none_match, &etag) {
        return (StatusCode::NOT_MODIFIED, headers).into_response();
    }
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    (headers, body).into_response()
}

/// Serializes `value`, tags it, and answers with [`PUBLIC_MAX_AGE_5_MIN`].
///
/// For handlers whose body is cheap to build on every request; the validator
/// still saves the transfer, which for a national start list is the expensive
/// part on a venue's cellular link.
pub fn cacheable_json<T: Serialize>(value: &T, request: &HeaderMap) -> Result<Response, AppError> {
    let body = Bytes::from(serde_json::to_vec(value).map_err(anyhow::Error::from)?);
    let etag = strong_etag(&body);
    Ok(json_response(
        body,
        etag,
        Some(PUBLIC_MAX_AGE_5_MIN),
        request.get(header::IF_NONE_MATCH),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etag_is_a_quoted_sha256_of_the_exact_body() {
        let tag = strong_etag(b"{}");
        let text = tag.to_str().unwrap();
        assert!(text.starts_with('"') && text.ends_with('"'));
        assert_eq!(text.len(), 66);
        assert_eq!(strong_etag(b"{}"), tag);
        assert_ne!(strong_etag(b"{ }"), tag);
    }

    #[test]
    fn if_none_match_accepts_exact_weak_and_star_but_nothing_else() {
        let tag = strong_etag(b"body");
        let hv = |value: &str| HeaderValue::from_str(value).unwrap();
        assert!(etag_matches(Some(&tag), &tag));
        assert!(etag_matches(
            Some(&hv(&format!("W/{}", tag.to_str().unwrap()))),
            &tag
        ));
        assert!(etag_matches(
            Some(&hv(&format!("\"other\", {}", tag.to_str().unwrap()))),
            &tag
        ));
        assert!(etag_matches(Some(&hv("*")), &tag));
        assert!(!etag_matches(Some(&hv("\"other\"")), &tag));
        assert!(!etag_matches(None, &tag));
        assert!(!etag_matches(Some(&strong_etag(b"other body")), &tag));
    }

    #[test]
    fn matching_validator_short_circuits_to_304_without_a_body() {
        let body = Bytes::from_static(b"{\"meet\":1}");
        let tag = strong_etag(&body);
        let response = json_response(body.clone(), tag.clone(), None, Some(&tag));
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(response.headers().get(header::ETAG), Some(&tag));
        assert!(response.headers().get(header::CACHE_CONTROL).is_none());

        let response = json_response(body, tag.clone(), Some(PUBLIC_MAX_AGE_5_MIN), None);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(header::ETAG), Some(&tag));
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            PUBLIC_MAX_AGE_5_MIN
        );
    }

    #[test]
    fn cacheable_json_revalidates_with_the_body_hash() {
        let value = serde_json::json!({"a": 1});
        let body = serde_json::to_vec(&value).unwrap();
        let tag = strong_etag(&body);
        let mut request = HeaderMap::new();
        request.insert(header::IF_NONE_MATCH, tag.clone());
        let response = cacheable_json(&value, &request).unwrap();
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            PUBLIC_MAX_AGE_5_MIN
        );
        let response = cacheable_json(&value, &HeaderMap::new()).unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(header::ETAG), Some(&tag));
    }
}
