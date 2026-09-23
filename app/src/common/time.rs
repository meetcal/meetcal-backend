use crate::AppError;
use std::time::{SystemTime, UNIX_EPOCH};

/// Wall-clock milliseconds since the UNIX epoch, used to stamp `updated_at` on
/// user-owned rows. A clock before the epoch is the server's fault, so it is a
/// 500, not a 400 blamed on the client, and never a panic.
pub fn now_millis() -> Result<i64, AppError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::Internal(anyhow::anyhow!("system clock is before UNIX epoch")))?;

    Ok(duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_millis_is_after_2020() {
        assert!(now_millis().unwrap() > 1_577_836_800_000);
    }
}
