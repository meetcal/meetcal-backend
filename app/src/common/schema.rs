//! Refuse to serve against a database that is behind this build.
//!
//! Migrations are applied by hand (`sqlx migrate run`, as the table owner),
//! while deploys happen on every green push to master. A new binary that
//! reached production before its migrations would 500 on every route that
//! needs them. Checking the ledger at startup turns that into a crash the
//! deploy's health check sees, so the previous container keeps serving.
//!
//! The same startup check covers the database's character locale. Name
//! matching runs `lower()` and a `\s` regex inside Postgres
//! (`normalized_name_sql!`, the `*_name_normalized` indexes) against a
//! parameter the app normalizes with Unicode rules (`normalize_name`). Under
//! `LC_CTYPE = C` / `POSIX`, Postgres folds only ASCII and `\s` skips
//! non-ASCII spaces, so `JOSÉ ÁLVAREZ` matches nothing and a no-break space
//! never collapses. Nothing else pins the locale, so refuse to start against
//! a database whose ctype cannot agree with the app.

use sqlx::PgPool;

/// Versions this binary was built against, from `app/migrations`.
pub fn embedded_migration_versions() -> Vec<i64> {
    sqlx::migrate!("./migrations")
        .iter()
        .map(|migration| migration.version)
        .collect()
}

/// Embedded versions the database has not successfully applied, ascending.
pub fn missing_migrations(expected: &[i64], applied: &[i64]) -> Vec<i64> {
    let mut missing: Vec<i64> = expected
        .iter()
        .copied()
        .filter(|version| !applied.contains(version))
        .collect();
    missing.sort_unstable();
    missing
}

/// `Ok` when every embedded migration is recorded as applied. The error says
/// what to run; it never carries driver text beyond the reason the ledger
/// could not be read.
pub async fn ensure_migrations_applied(db: &PgPool) -> Result<(), String> {
    let applied: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success")
            .fetch_all(db)
            .await
            .map_err(|error| {
                format!(
                    "cannot read _sqlx_migrations ({error}); run `sqlx migrate run` as the \
                     table owner so the API role is granted SELECT on it"
                )
            })?;
    let missing = missing_migrations(&embedded_migration_versions(), &applied);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "database is missing migrations {missing:?}; run `sqlx migrate run` before \
             starting this build"
        ))
    }
}

/// Character encoding the API requires: names are stored and compared as
/// UTF-8 on the app side, and every other encoding would mojibake them.
pub const REQUIRED_ENCODING: &str = "UTF8";

/// The libc ctypes that fold only ASCII. Anything else (`en_US.UTF-8`,
/// `C.UTF-8`, ICU) folds and classifies the whole of Unicode as the app does.
const ASCII_ONLY_CTYPES: [&str; 2] = ["C", "POSIX"];

/// Locale facts about the connected database, from `pg_database`.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct DatabaseLocale {
    /// `pg_encoding_to_char(encoding)`, e.g. `UTF8`.
    pub encoding: String,
    /// `datlocprovider`: `c` (libc), `i` (ICU), or `b` (builtin, Postgres 17+).
    pub provider: String,
    /// `datctype`, the libc `LC_CTYPE`. Still set for the ICU provider, which
    /// does not use it for `lower()` or regex classes.
    pub ctype: String,
}

/// `Some(reason)` when the app's Unicode name rule cannot agree with this
/// database's `lower()` / `\s`. Pure so it is unit-tested against every
/// provider without a cluster per locale.
pub fn locale_problem(locale: &DatabaseLocale) -> Option<String> {
    if !locale.encoding.eq_ignore_ascii_case(REQUIRED_ENCODING) {
        return Some(format!(
            "database encoding is {} but the API requires {REQUIRED_ENCODING}; recreate the \
             database with `ENCODING 'UTF8'`",
            locale.encoding
        ));
    }
    // ICU folds by Unicode rules regardless of `datctype`. libc and the
    // builtin provider both classify characters by the ctype.
    if locale.provider == "i" {
        return None;
    }
    let ctype = locale.ctype.trim();
    if ASCII_ONLY_CTYPES
        .iter()
        .any(|ascii_only| ctype.eq_ignore_ascii_case(ascii_only))
    {
        return Some(format!(
            "database LC_CTYPE is {ctype:?}, which folds only ASCII: Postgres lower() and \
             regex \\s would disagree with the app's Unicode name normalization, so \
             non-ASCII names (JOSÉ ÁLVAREZ) would match nothing. Recreate the database \
             with a UTF-8 ctype (`LC_CTYPE 'C.UTF-8'` or `'en_US.UTF-8'`, or the ICU \
             provider); LC_CTYPE cannot be changed on an existing database"
        ));
    }
    None
}

/// What `pg_database` says about the connected database.
pub async fn database_locale(db: &PgPool) -> Result<DatabaseLocale, String> {
    sqlx::query_as(
        "SELECT pg_encoding_to_char(encoding) AS encoding, \
                datlocprovider::text AS provider, \
                datctype AS ctype \
         FROM pg_database WHERE datname = current_database()",
    )
    .fetch_one(db)
    .await
    .map_err(|error| format!("cannot read the database locale from pg_database ({error})"))
}

/// `Ok` when the database encoding is UTF-8 and its ctype folds Unicode
/// ([`locale_problem`]). Read at startup next to the migration ledger.
pub async fn ensure_database_locale(db: &PgPool) -> Result<(), String> {
    let locale = database_locale(db).await?;
    match locale_problem(&locale) {
        None => Ok(()),
        Some(reason) => Err(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locale(encoding: &str, provider: &str, ctype: &str) -> DatabaseLocale {
        DatabaseLocale {
            encoding: encoding.to_string(),
            provider: provider.to_string(),
            ctype: ctype.to_string(),
        }
    }

    #[test]
    fn utf8_unicode_ctypes_are_accepted() {
        for (provider, ctype) in [
            ("c", "en_US.UTF-8"),
            ("c", "en_US.utf8"),
            ("c", "C.UTF-8"),
            ("c", "C.utf8"),
            ("b", "C.UTF-8"),
            ("i", "C"),
        ] {
            assert_eq!(
                locale_problem(&locale("UTF8", provider, ctype)),
                None,
                "{provider} {ctype}"
            );
        }
    }

    #[test]
    fn ascii_only_ctypes_are_refused_with_the_fix() {
        for (provider, ctype) in [("c", "C"), ("c", "POSIX"), ("c", " c "), ("b", "C")] {
            let reason = locale_problem(&locale("UTF8", provider, ctype))
                .unwrap_or_else(|| panic!("{provider} {ctype} must be refused"));
            assert!(reason.contains("folds only ASCII"), "{reason}");
            assert!(reason.contains("C.UTF-8"), "{reason}");
        }
    }

    #[test]
    fn non_utf8_encodings_are_refused_whatever_the_ctype() {
        let reason = locale_problem(&locale("LATIN1", "c", "en_US")).expect("refused");
        assert!(
            reason.contains("LATIN1") && reason.contains("UTF8"),
            "{reason}"
        );
        let reason = locale_problem(&locale("SQL_ASCII", "i", "C")).expect("refused");
        assert!(reason.contains("SQL_ASCII"), "{reason}");
    }

    #[test]
    fn reports_only_unapplied_versions_in_order() {
        assert_eq!(missing_migrations(&[3, 1, 2], &[1]), vec![2, 3]);
        assert!(missing_migrations(&[1, 2], &[2, 1, 99]).is_empty());
    }

    #[test]
    fn embeds_every_migration_file() {
        let versions = embedded_migration_versions();
        assert!(
            versions.windows(2).all(|pair| pair[0] < pair[1]),
            "{versions:?}"
        );
        assert!(versions.contains(&20260923300000));
    }
}
