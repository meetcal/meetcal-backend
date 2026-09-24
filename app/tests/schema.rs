//! Startup checks against the real database: the migration ledger and the
//! locale that name matching depends on.
use app::common::schema::{
    DatabaseLocale, database_locale, ensure_database_locale, ensure_migrations_applied,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::str::FromStr;

mod support;

/// A database created with the ASCII-only libc ctype, to prove the check
/// refuses it. Created once by whichever test gets there first; never dropped,
/// so a parallel test never sees it half-built.
const ASCII_ONLY_DATABASE: &str = "meetcal_test_ctype_c";

#[tokio::test]
async fn the_test_database_passes_the_startup_checks() {
    let db = support::db_pool().await;
    ensure_migrations_applied(&db).await.expect("migrated");
    let locale = database_locale(&db).await.expect("locale readable");
    assert_eq!(locale.encoding, "UTF8", "{locale:?}");
    ensure_database_locale(&db)
        .await
        .unwrap_or_else(|reason| panic!("the test database must fold Unicode: {reason}"));
}

#[tokio::test]
async fn the_api_role_can_read_the_locale() {
    let db = support::db_pool().await;
    let mut conn = db.acquire().await.unwrap();
    sqlx::query("SET ROLE meetcal_api")
        .execute(&mut *conn)
        .await
        .unwrap();
    let locale: DatabaseLocale = sqlx::query_as(
        "SELECT pg_encoding_to_char(encoding) AS encoding, datlocprovider::text AS provider, \
         datctype AS ctype FROM pg_database WHERE datname = current_database()",
    )
    .fetch_one(&mut *conn)
    .await
    .expect("pg_database is readable by every role");
    assert_eq!(locale.encoding, "UTF8");
}

#[tokio::test]
async fn an_ascii_only_ctype_is_refused_at_startup() {
    let db = support::db_pool().await;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(ASCII_ONLY_DATABASE)
            .fetch_one(&db)
            .await
            .unwrap();
    if !exists {
        // `CREATE DATABASE` takes no parameters and cannot run in a transaction.
        let created = sqlx::query(
            "CREATE DATABASE meetcal_test_ctype_c TEMPLATE template0 ENCODING 'UTF8' \
             LC_COLLATE 'C' LC_CTYPE 'C'",
        )
        .execute(&db)
        .await;
        if let Err(error) = created {
            // Another test binary may have raced us to it; anything else
            // (no CREATEDB privilege) is a real failure.
            let message = error.to_string();
            assert!(
                message.contains("already exists"),
                "cannot create the ASCII-only database: {message}"
            );
        }
    }

    let url = support::database_url();
    let options = PgConnectOptions::from_str(&url)
        .expect("DATABASE_URL parses")
        .database(ASCII_ONLY_DATABASE);
    let ascii_only = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect to the ASCII-only database");

    let locale = database_locale(&ascii_only).await.unwrap();
    assert_eq!(locale.ctype, "C", "{locale:?}");
    // The symptom the check exists for: Postgres leaves É alone here.
    let folded: String = sqlx::query_scalar("SELECT lower('JOSÉ ÁLVAREZ')")
        .fetch_one(&ascii_only)
        .await
        .unwrap();
    assert_eq!(folded, "josÉ Álvarez");

    let reason = ensure_database_locale(&ascii_only)
        .await
        .expect_err("a C ctype must refuse to start");
    assert!(reason.contains("folds only ASCII"), "{reason}");
    assert!(reason.contains("C.UTF-8"), "{reason}");
}
