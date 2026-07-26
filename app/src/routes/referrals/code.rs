//! Referral invite-code generation and lookup.

use crate::AppError;
use sqlx::{Postgres, Transaction};

/// Unambiguous alphabet: A-Z and 2-9 with the visually confusable characters
/// (`0`, `1`, `I`, `O`) removed. 32 symbols.
const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 8;

/// Generate a random 8-char code from the unambiguous alphabet. Uses UUIDv4
/// bytes as the entropy source (no extra rand dependency).
pub fn generate_code() -> String {
    let mut out = String::with_capacity(CODE_LEN);
    // 16 random bytes; we take one byte per output char.
    let bytes = uuid::Uuid::new_v4().into_bytes();
    for &b in bytes.iter().take(CODE_LEN) {
        out.push(ALPHABET[(b as usize) % ALPHABET.len()] as char);
    }
    out
}

/// Return the caller's referral code, creating it on first call. Runs under the
/// caller's RLS context (the row's `user_id` is the caller).
pub async fn ensure_referral_code(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
) -> Result<String, AppError> {
    // Fast path: already has one.
    if let Some(code) =
        sqlx::query_scalar::<_, String>("SELECT code FROM referral_codes WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await?
    {
        return Ok(code);
    }

    // Create one, retrying on the (astronomically unlikely) code collision.
    for _ in 0..8 {
        let candidate = generate_code();
        let inserted = sqlx::query_scalar::<_, String>(
            r#"
            INSERT INTO referral_codes (user_id, code)
            VALUES ($1, $2)
            ON CONFLICT (user_id) DO NOTHING
            RETURNING code
            "#,
        )
        .bind(user_id)
        .bind(&candidate)
        .fetch_optional(&mut **tx)
        .await;

        match inserted {
            Ok(Some(code)) => return Ok(code),
            // user_id conflict (another concurrent insert won): read it back.
            Ok(None) => {
                if let Some(code) = sqlx::query_scalar::<_, String>(
                    "SELECT code FROM referral_codes WHERE user_id = $1",
                )
                .bind(user_id)
                .fetch_optional(&mut **tx)
                .await?
                {
                    return Ok(code);
                }
            }
            // Only a UNIQUE violation (SQLSTATE 23505) on the code column is a
            // retryable collision; anything else is a real error to propagate.
            Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
                continue;
            }
            Err(other) => return Err(other.into()),
        }
    }

    Err(AppError::Validation(
        "could not allocate a referral code".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_shape() {
        for _ in 0..100 {
            let c = generate_code();
            assert_eq!(c.len(), CODE_LEN);
            assert!(c.bytes().all(|b| ALPHABET.contains(&b)));
            // No ambiguous characters.
            assert!(!c.contains(['0', '1', 'I', 'O']));
        }
    }
}
