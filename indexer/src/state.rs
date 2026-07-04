use std::sync::Arc;

use tokio::sync::Mutex;

/// Live progress of the current run, shared between the processor and the
/// cursor-promotion task.
#[derive(Clone)]
pub struct Session {
    /// Newest processed `(signature, slot)`.
    pub head: Option<(String, i64)>,
    /// Oldest slot processed this run — the backfill frontier. Descends while
    /// backfilling; goes STABLE once the backlog is drained (caught up).
    pub min_slot: i64,
}

impl Default for Session {
    fn default() -> Self {
        Session { head: None, min_slot: i64::MAX }
    }
}

pub type SharedSession = Arc<Mutex<Session>>;

pub fn shared_session() -> SharedSession {
    Arc::new(Mutex::new(Session::default()))
}

/// Whether the durable cursor may be promoted this tick: only once the backfill
/// frontier (oldest slot processed) has gone STABLE across a tick — meaning the
/// backlog has drained and we've caught up. Advancing the cursor mid-backfill
/// would risk skipping the still-un-backfilled older range on a crash.
pub fn frontier_stable(prev_min: i64, cur_min: i64) -> bool {
    cur_min != i64::MAX && cur_min == prev_min
}

#[cfg(test)]
mod tests {
    use super::frontier_stable;

    #[test]
    fn promotes_only_when_frontier_is_stable() {
        // Backfilling: frontier still descending → do not promote.
        assert!(!frontier_stable(500, 400));
        // Nothing processed yet → do not promote.
        assert!(!frontier_stable(i64::MAX, i64::MAX));
        // Drained / caught up: frontier unchanged across a tick → promote.
        assert!(frontier_stable(400, 400));
    }
}
