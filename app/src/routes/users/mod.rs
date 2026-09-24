pub mod auth;
pub mod preferences;
pub mod saved_sessions;

use crate::common::query::MAX_SAVED_SESSION_ATHLETE_NAMES;
use saved_sessions::{
    MAX_SAVED_SESSION_ATHLETE_NAME_LEN, MAX_SAVED_SESSION_FIELD_LEN, MAX_SAVED_SESSION_MEET_LEN,
    MAX_SAVED_SESSION_NOTES_LEN,
};

/// Request-body ceiling for the `/users/me/*` write routes (`PUT` saved
/// session, `PATCH` preferences). The largest valid body is a saved session at
/// every field cap: notes (2,000 chars), 64 athlete names of 128 chars, the
/// meet (256) and four short fields (64 each), about 10,700 characters, which
/// is at most ~43 KB even if every character is a 4-byte UTF-8 sequence. 64 KiB
/// admits every body that can pass validation and rejects anything larger with
/// `413` before it is buffered.
pub const USER_WRITE_BODY_LIMIT: usize = 64 * 1024;
const _: () = assert!(
    4 * (MAX_SAVED_SESSION_NOTES_LEN
        + MAX_SAVED_SESSION_ATHLETE_NAMES * MAX_SAVED_SESSION_ATHLETE_NAME_LEN
        + MAX_SAVED_SESSION_MEET_LEN
        + 4 * MAX_SAVED_SESSION_FIELD_LEN)
        + 2048
        <= USER_WRITE_BODY_LIMIT
);
