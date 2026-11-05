use std::time::{Duration, Instant};

use axum::http::StatusCode;
use tokio::task::JoinSet;
use uuid::Uuid;

use super::test_helpers::{caller, poll, position, sync, user, warm, without_postgres};
use crate::modules::chat::sync::bump;
use crate::modules::chat::sync::plan::{HOLD_JITTER, jittered_hold};

const PARKS: usize = 30;
const HOLD_MS: u64 = 1_000;

#[test]
fn a_hold_ends_at_most_a_tenth_early() {
    let hold = Duration::from_secs(20);
    let ends: Vec<Duration> = (0..10_000).map(|_| jittered_hold(hold)).collect();
    let shortest = hold.mul_f64(1.0 - HOLD_JITTER);
    assert!(ends.iter().all(|end| *end <= hold && *end >= shortest));
    assert!(
        ends.iter()
            .min()
            .is_some_and(|end| *end < Duration::from_millis(18_200))
    );
    assert!(
        ends.iter()
            .max()
            .is_some_and(|end| *end > Duration::from_millis(19_800))
    );
    assert!(jittered_hold(Duration::ZERO).is_zero());
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn polls_parked_together_end_spread_inside_their_hold() {
    let chat = without_postgres();
    let users: Vec<String> = (0..PARKS).map(|_| user()).collect();
    let names: Vec<&str> = users.iter().map(String::as_str).collect();
    let incarnation = warm(&chat, &names);
    let mut polls = JoinSet::new();
    for user in users {
        bump::devices(&chat, &user).await.expect("a bump");
        let pos = position(&chat, &user).await;
        let chat = chat.clone();
        polls.spawn(async move {
            let started = Instant::now();
            let answer = sync(
                &chat,
                caller(&user, Uuid::now_v7()),
                poll(pos, incarnation, HOLD_MS),
            )
            .await;
            (answer.status, started.elapsed())
        });
    }
    let mut ended = Vec::with_capacity(PARKS);
    while let Some(done) = polls.join_next().await {
        let (status, elapsed) = done.expect("a poll");
        assert_eq!(status, StatusCode::NO_CONTENT);
        ended.push(elapsed);
    }

    let hold = Duration::from_millis(HOLD_MS);
    let first = ended.iter().min().copied().unwrap_or_default();
    let last = ended.iter().max().copied().unwrap_or_default();
    assert!(first >= hold.mul_f64(1.0 - HOLD_JITTER), "{first:?}");
    assert!(last < hold + Duration::from_millis(300), "{last:?}");
    assert!(
        last - first >= Duration::from_millis(30),
        "{PARKS} polls ended within {:?}",
        last - first
    );
}
