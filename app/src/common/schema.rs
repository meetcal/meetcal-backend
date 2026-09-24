//! Refuse to serve against a database that is behind this build.
//!
//! Migrations are applied by hand (`sqlx migrate run`, as the table owner),
//! while deploys happen on every green push to master. A new binary that
//! reached production before its migrations would 500 on every route that
//! needs them. Checking the ledger at startup turns that into a crash the
//! deploy's health check sees, so the previous container keeps serving.

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

#[cfg(test)]
mod tests {
    use super::*;

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
