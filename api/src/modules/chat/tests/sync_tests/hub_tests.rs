use std::time::Duration;

use tokio::time::{Instant, advance, timeout};
use uuid::Uuid;

use crate::modules::chat::hub::{COALESCE, ChatHub};

const QUIET: Duration = Duration::from_millis(50);

fn device(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[tokio::test(start_paused = true)]
async fn the_first_wake_fires_at_once_and_later_ones_wait_for_the_window() {
    let hub = ChatHub::new();
    let mut parked = hub.park("101", device(1), None);
    let start = Instant::now();

    hub.wake("101");
    assert_eq!(parked.next().await.at, Some(start));

    advance(Duration::from_millis(100)).await;
    hub.wake("101");
    advance(Duration::from_millis(100)).await;
    hub.wake("101");
    let coalesced = parked.next().await;
    assert_eq!(coalesced.at, Some(start + COALESCE));
    assert_eq!(coalesced.woken, Some(start + Duration::from_millis(100)));

    advance(COALESCE * 3).await;
    hub.wake("101");
    assert_eq!(parked.next().await.at, Some(Instant::now()));
}

#[tokio::test(start_paused = true)]
async fn the_window_is_per_user_and_shared_by_every_device_of_that_user() {
    let hub = ChatHub::new();
    let mut phone = hub.park("101", device(1), None);
    let mut laptop = hub.park("101", device(2), None);
    let mut stranger = hub.park("202", device(3), None);
    let start = Instant::now();

    hub.wake("101");
    assert_eq!(phone.next().await.at, Some(start));
    assert_eq!(laptop.next().await.at, Some(start));
    assert!(timeout(QUIET, stranger.next()).await.is_err());

    let later = Instant::now();
    hub.wake("202");
    assert_eq!(stranger.next().await.at, Some(later));
}

#[tokio::test(start_paused = true)]
async fn a_taken_signal_is_cleared_until_the_next_wake() {
    let hub = ChatHub::new();
    let mut parked = hub.park("101", device(1), None);
    hub.wake("101");
    assert!(parked.next().await.at.is_some());
    assert!(timeout(QUIET, parked.next()).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn typing_wakes_only_polls_focused_on_that_conversation() {
    let hub = ChatHub::new();
    let conversation = Uuid::now_v7();
    let mut focused = hub.park("202", device(1), Some(conversation));
    let mut elsewhere = hub.park("202", device(2), Some(Uuid::now_v7()));
    let mut unfocused = hub.park("303", device(3), None);
    let mut typer = hub.park("101", device(4), Some(conversation));

    hub.wake_focus(conversation, "101");

    let signal = timeout(QUIET, focused.next())
        .await
        .expect("the focused poll wakes");
    assert!(signal.focus);
    assert!(timeout(QUIET, elsewhere.next()).await.is_err());
    assert!(timeout(QUIET, unfocused.next()).await.is_err());
    assert!(timeout(QUIET, typer.next()).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn a_data_wake_reaches_focused_polls_without_the_focus_flag() {
    let hub = ChatHub::new();
    let mut focused = hub.park("202", device(1), Some(Uuid::now_v7()));
    hub.wake("202");
    let signal = focused.next().await;
    assert!(signal.at.is_some());
    assert!(!signal.focus);
}

#[tokio::test(start_paused = true)]
async fn the_newest_poll_of_a_device_ends_the_older_one() {
    let hub = ChatHub::new();
    let mut older = hub.park("101", device(1), None);
    let other = hub.park("101", device(2), None);
    let newer = hub.park("101", device(1), None);

    assert!(older.next().await.superseded);
    assert!(older.superseded());
    assert!(!other.superseded());
    assert!(!newer.superseded());
}

#[tokio::test(start_paused = true)]
async fn parked_counts_follow_the_guards_and_idle_slots_are_pruned() {
    let hub = ChatHub::new();
    let conversation = Uuid::now_v7();
    {
        let _first = hub.park("101", device(1), Some(conversation));
        let _second = hub.park("202", device(2), None);
        assert_eq!(hub.parked(), 2);
    }
    assert_eq!(hub.parked(), 0);
    let _kept = hub.park("303", device(3), None);
    assert_eq!(hub.tracked_users(), 3);

    advance(Duration::from_secs(31)).await;
    hub.prune_all();
    assert_eq!(hub.tracked_users(), 1);

    hub.wake_focus(conversation, "nobody");
}

#[tokio::test(start_paused = true)]
async fn twenty_immediate_answers_then_the_device_waits_for_refill() {
    let hub = ChatHub::new();
    for _ in 0..20 {
        assert!(hub.take_token("101", device(1)));
    }
    assert!(!hub.take_token("101", device(1)));
    assert!(hub.take_token("101", device(2)));

    advance(Duration::from_millis(500)).await;
    assert!(hub.take_token("101", device(1)));
    assert!(!hub.take_token("101", device(1)));

    advance(Duration::from_secs(10)).await;
    for _ in 0..20 {
        assert!(hub.take_token("101", device(1)));
    }
    assert!(!hub.take_token("101", device(1)));
}

#[tokio::test(start_paused = true)]
async fn presence_is_marked_at_most_once_a_minute_per_user() {
    let hub = ChatHub::new();
    assert!(hub.seen_due("101"));
    assert!(!hub.seen_due("101"));
    assert!(hub.seen_due("202"));
    advance(Duration::from_secs(61)).await;
    assert!(hub.seen_due("101"));
}

#[tokio::test(start_paused = true)]
async fn a_verified_focus_belongs_to_one_device() {
    let hub = ChatHub::new();
    let conversation = Uuid::now_v7();
    assert_eq!(hub.focus_of("101", device(1)), None);
    hub.set_focus("101", device(1), Some(conversation));
    assert_eq!(hub.focus_of("101", device(1)), Some(conversation));
    assert_eq!(hub.focus_of("101", device(2)), None);
    hub.set_focus("101", device(1), None);
    assert_eq!(hub.focus_of("101", device(1)), None);
}

#[tokio::test(start_paused = true)]
async fn the_typing_board_forgets_typers_after_six_seconds() {
    let hub = ChatHub::new();
    let conversation = Uuid::now_v7();
    let now = Instant::now();
    hub.typing.note(conversation, "101", now);
    assert!(hub.typing.repeated(conversation, "101", now));
    assert_eq!(hub.typing.candidates(conversation, now), vec!["101"]);

    advance(Duration::from_secs(3)).await;
    assert!(!hub.typing.repeated(conversation, "101", Instant::now()));
    hub.typing.note(conversation, "202", Instant::now());
    advance(Duration::from_secs(4)).await;
    assert_eq!(
        hub.typing.candidates(conversation, Instant::now()),
        vec!["202"]
    );
}
