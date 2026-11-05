use std::collections::HashMap;
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::{CALLER_TTL, Callers, SessionEntry};
use crate::modules::chat::held::FRESH_SPREAD;

const FILLED: usize = 5_000;

fn entry() -> SessionEntry {
    SessionEntry {
        user_id: "101".to_owned(),
        bind: None,
    }
}

#[test]
fn sessions_cached_together_go_stale_over_minutes_not_in_one_second() {
    let callers = Callers::new();
    let ids: Vec<Uuid> = (0..FILLED).map(|_| Uuid::new_v4()).collect();
    let filled = Instant::now();
    for id in &ids {
        callers.remember_session(*id, entry());
    }
    let done = Instant::now();

    let earliest = filled + CALLER_TTL.mul_f64(1.0 - FRESH_SPREAD);
    let mut per_second: HashMap<u64, usize> = HashMap::new();
    for id in &ids {
        let held = callers.session(*id).expect("a held session");
        assert!(held.fresh());
        assert!(held.fresh_until >= earliest && held.fresh_until <= done + CALLER_TTL);
        *per_second
            .entry(held.fresh_until.duration_since(filled).as_secs())
            .or_default() += 1;
    }
    let busiest = per_second.values().copied().max().unwrap_or_default();
    assert!(per_second.len() >= 100, "{} seconds", per_second.len());
    assert!(
        busiest <= 125,
        "{busiest} of {FILLED} sessions go stale in one second"
    );
}

#[test]
fn a_forget_drops_a_stale_session_at_once() {
    let callers = Callers::with_freshness(Duration::ZERO);
    let id = Uuid::new_v4();
    callers.remember_session(id, entry());
    let held = callers.session(id).expect("a stale session is still held");
    assert!(!held.fresh());

    callers.forget_session(id);
    assert!(callers.session(id).is_none());
}
