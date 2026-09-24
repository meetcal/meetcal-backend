//! Client version signal.
//!
//! The mobile app sends `X-MeetCal-App: <major>.<minor>.<patch>` on every
//! request. Shipped app builds cannot be patched in lockstep with the API, so
//! stricter validation is gated on this header instead of flipped globally: a
//! client at or above [`MIN_STRICT_CLIENT_VERSION`] fails closed with `400`,
//! while an older client — or one that sends no header at all — keeps the
//! legacy behaviour it was built against. Raise the threshold when a new app
//! version starts depending on a stricter contract; remove a legacy branch only
//! once the version tail that needs it is gone.
use crate::{AppError, common::query};
use axum::{extract::FromRequestParts, http::request::Parts};
use std::convert::Infallible;

/// Header name, lowercase as `http` stores it.
pub const CLIENT_VERSION_HEADER: &str = "x-meetcal-app";

/// First app version that opts into fail-closed validation.
pub const MIN_STRICT_CLIENT_VERSION: Version = Version {
    major: 6,
    minor: 2,
    patch: 0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// Parses the leading `major[.minor[.patch]]` of a version string.
    ///
    /// A trailing build suffix (`6.2.0 (123)`, `6.2.0-beta`) is ignored and a
    /// missing minor or patch reads as `0`. Anything that does not start with
    /// a number is `None`, which callers treat as "legacy client".
    pub fn parse(raw: &str) -> Option<Self> {
        let numeric: String = raw
            .trim()
            .chars()
            .take_while(|character| character.is_ascii_digit() || *character == '.')
            .collect();
        let parts: Vec<&str> = numeric
            .split('.')
            .filter(|part| !part.is_empty())
            .take(3)
            .collect();
        let component = |index: usize| -> Option<u32> {
            match parts.get(index) {
                None => Some(0),
                Some(part) => part.parse().ok(),
            }
        };
        Some(Self {
            major: parts.first()?.parse().ok()?,
            minor: component(1)?,
            patch: component(2)?,
        })
    }
}

/// The calling client's declared version, `None` for legacy or non-app callers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientVersion(pub Option<Version>);

impl ClientVersion {
    /// Whether this client has opted into fail-closed validation.
    pub fn strict(&self) -> bool {
        self.0
            .is_some_and(|version| version >= MIN_STRICT_CLIENT_VERSION)
    }

    /// Strict clients get `400` on an empty required field; legacy clients keep
    /// the historical `200 []` answer.
    pub fn require_non_empty(&self, field: &str, value: &str) -> Result<(), AppError> {
        if self.strict() {
            query::require_non_empty(field, value)
        } else {
            Ok(())
        }
    }

    /// Strict clients get `400` on a malformed optional date; legacy clients
    /// keep the historical string comparison.
    pub fn require_iso_date(&self, field: &str, value: Option<&str>) -> Result<(), AppError> {
        if self.strict() {
            query::require_iso_date(field, value)
        } else {
            Ok(())
        }
    }

    /// Strict clients get `400` on a `year` that is not four digits; legacy
    /// clients keep the historical string comparison against `YYYY-01-01`.
    pub fn require_year(&self, field: &str, value: &str) -> Result<(), AppError> {
        if self.strict() {
            query::require_year(field, value)
        } else {
            Ok(())
        }
    }

    /// Strict clients must send the date and it must be valid; legacy clients
    /// may omit it and fall back to the server-side default window.
    pub fn require_present_iso_date(
        &self,
        field: &str,
        value: Option<&str>,
    ) -> Result<(), AppError> {
        if !self.strict() {
            return Ok(());
        }
        match value {
            None => Err(AppError::Validation(format!("{field} is required"))),
            Some(value) => query::require_iso_date(field, Some(value)),
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientVersion {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let version = parts
            .headers
            .get(CLIENT_VERSION_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(Version::parse);
        Ok(Self(version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(major: u32, minor: u32, patch: u32) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn parses_full_short_and_suffixed_versions() {
        assert_eq!(Version::parse("6.2.0"), Some(version(6, 2, 0)));
        assert_eq!(Version::parse(" 6.2.1 "), Some(version(6, 2, 1)));
        assert_eq!(Version::parse("6.2"), Some(version(6, 2, 0)));
        assert_eq!(Version::parse("7"), Some(version(7, 0, 0)));
        assert_eq!(Version::parse("6.2.0 (123)"), Some(version(6, 2, 0)));
        assert_eq!(Version::parse("6.2.0-beta.1"), Some(version(6, 2, 0)));
        assert_eq!(Version::parse("6.10.0"), Some(version(6, 10, 0)));
    }

    #[test]
    fn rejects_non_numeric_versions() {
        assert_eq!(Version::parse(""), None);
        assert_eq!(Version::parse("latest"), None);
        assert_eq!(Version::parse("v6.2.0"), None);
        assert_eq!(Version::parse("."), None);
    }

    #[test]
    fn strictness_is_a_numeric_threshold_not_a_string_compare() {
        let strict = |raw: &str| ClientVersion(Version::parse(raw)).strict();
        assert!(!ClientVersion(None).strict());
        assert!(!strict("6.1.9"));
        assert!(!strict("5.9.9"));
        assert!(strict("6.2.0"));
        assert!(strict("6.2.1"));
        assert!(strict("6.10.0"));
        assert!(strict("7.0.0"));
    }

    #[test]
    fn legacy_clients_are_never_rejected() {
        let legacy = ClientVersion(None);
        assert!(legacy.require_non_empty("wso", "").is_ok());
        assert!(legacy.require_iso_date("cutoff_date", Some("nope")).is_ok());
        assert!(legacy.require_present_iso_date("cutoff_date", None).is_ok());
        assert!(legacy.require_year("year", "abc").is_ok());
    }

    #[test]
    fn strict_clients_fail_closed() {
        let strict = ClientVersion(Some(MIN_STRICT_CLIENT_VERSION));
        assert!(strict.require_non_empty("wso", "  ").is_err());
        assert!(strict.require_non_empty("wso", "Carolina").is_ok());
        assert!(strict.require_year("year", "abc").is_err());
        assert!(strict.require_year("year", "2026").is_ok());
        assert!(strict.require_iso_date("cutoff_date", None).is_ok());
        assert!(
            strict
                .require_iso_date("cutoff_date", Some("2025-02-30"))
                .is_err()
        );
        assert!(
            strict
                .require_present_iso_date("cutoff_date", None)
                .is_err()
        );
        assert!(
            strict
                .require_present_iso_date("cutoff_date", Some("2025-01-01"))
                .is_ok()
        );
    }
}
