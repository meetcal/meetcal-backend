pub mod auth;
pub mod preferences;
pub mod saved_sessions;

use crate::common::query::{MAX_JSON_BYTES_PER_CHAR, MAX_SAVED_SESSION_ATHLETE_NAMES};
use saved_sessions::{
    MAX_SAVED_SESSION_ATHLETE_NAME_LEN, MAX_SAVED_SESSION_FIELD_LEN, MAX_SAVED_SESSION_MEET_LEN,
    MAX_SAVED_SESSION_NOTES_LEN,
};

/// Request-body ceiling for the `/users/me/*` write routes (`PUT` saved
/// session, `PATCH` preferences). The largest valid body is a saved session at
/// every field cap: notes (2,000 chars), 64 athlete names of 128 chars, the
/// meet (256) and four short fields (64 each), 10,704 characters. The caps count
/// decoded characters, and an ASCII-only serializer can send each one as a
/// 12-byte surrogate-pair escape, so that is up to ~128 KB on the wire. 160 KiB
/// admits every body that can pass validation however it is encoded, and
/// rejects anything larger with `413` before it is buffered.
pub const USER_WRITE_BODY_LIMIT: usize = 160 * 1024;
const _: () = assert!(
    MAX_JSON_BYTES_PER_CHAR
        * (MAX_SAVED_SESSION_NOTES_LEN
            + MAX_SAVED_SESSION_ATHLETE_NAMES * MAX_SAVED_SESSION_ATHLETE_NAME_LEN
            + MAX_SAVED_SESSION_MEET_LEN
            + 4 * MAX_SAVED_SESSION_FIELD_LEN)
        + 2048
        <= USER_WRITE_BODY_LIMIT
);
