//! Moderation on the game server (LR-15). Sanctions arrive with the career
//! view (`CareerPort::sanctions`) and are enforced where they apply: sign-in
//! (`career_backend`), `FindMatch` (`career_runtime`), chat (`social`) and
//! party presence (`party`). Every check compares a sanction's end with the
//! wall clock, so an expired one stops applying at once and a lifted one at
//! the next access refresh, without a restart.

/// The wall clock in Unix seconds, as sanctions measure their end.
pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().min(i64::MAX as u64) as i64)
}
