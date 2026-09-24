//! Server-wide load shedding.
//!
//! At most `max_in_flight` requests run at once (default
//! [`crate::DEFAULT_MAX_IN_FLIGHT`]). One more is answered at once with
//! `503 {"error":"overloaded"}` and `Retry-After: 1` instead of queueing on the
//! Postgres pool, where it would wait out `DB_ACQUIRE_TIMEOUT` and fail anyway
//! while holding memory and a socket. This always applies, independent of the
//! per-client limits' enforce setting. [`HEALTH_PATH`] is exempt so the deploy
//! check and uptime monitor still see a live process under load.
use crate::{AppError, common::rate_limit::HEALTH_PATH};
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

/// At most one "shedding load" warning per interval, however many requests
/// are shed.
pub const SHED_LOG_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct LoadShedder {
    slots: Arc<Semaphore>,
    max_in_flight: usize,
    log_gate: DefaultDirectRateLimiter,
}

impl LoadShedder {
    /// `max_in_flight` must be at least 1: a server that sheds everything is a
    /// configuration mistake, not a limit.
    pub fn new(max_in_flight: usize) -> anyhow::Result<Self> {
        anyhow::ensure!(
            (1..=Semaphore::MAX_PERMITS).contains(&max_in_flight),
            "rate_limit.max_in_flight must be at least 1"
        );
        let log_quota = Quota::with_period(SHED_LOG_INTERVAL)
            .ok_or_else(|| anyhow::anyhow!("SHED_LOG_INTERVAL must be non-zero"))?;
        Ok(Self {
            slots: Arc::new(Semaphore::new(max_in_flight)),
            max_in_flight,
            log_gate: RateLimiter::direct(log_quota),
        })
    }

    /// Requests currently holding a slot.
    pub fn in_flight(&self) -> usize {
        self.max_in_flight - self.slots.available_permits()
    }
}

/// Middleware: holds one slot for the life of the handler, or sheds the
/// request when none is free.
pub async fn shed_load(
    State(shedder): State<Arc<LoadShedder>>,
    request: Request,
    next: Next,
) -> Response {
    if request.uri().path() == HEALTH_PATH {
        return next.run(request).await;
    }
    match shedder.slots.clone().try_acquire_owned() {
        Ok(slot) => {
            let response = next.run(request).await;
            drop(slot);
            response
        }
        Err(_full) => {
            if shedder.log_gate.check().is_ok() {
                tracing::warn!(
                    max_in_flight = shedder.max_in_flight,
                    path = %request.uri().path(),
                    "in-flight request cap reached; shedding with 503"
                );
            }
            AppError::Overloaded.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_cap_is_rejected() {
        assert!(LoadShedder::new(0).is_err());
        let shedder = LoadShedder::new(3).unwrap();
        assert_eq!(shedder.in_flight(), 0);
    }
}
