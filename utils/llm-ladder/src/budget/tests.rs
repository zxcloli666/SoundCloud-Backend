use std::sync::Arc;

use chrono::TimeZone;

use super::memory::MemorySpend;
use super::*;

fn moment() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 9, 23, 59, 59).unwrap()
}

fn caps() -> Caps {
    Caps {
        month: Some(1_000_000),
        day: Some(200_000),
        user_month: Some(50_000),
        user_day: Some(10_000),
    }
}

fn budget(store: &Arc<MemorySpend>, caps: Caps) -> Arc<Budget> {
    Arc::new(Budget::new(store.clone(), "chat", caps))
}

#[tokio::test]
async fn a_charge_lands_on_every_capped_period_of_the_ladder_and_the_user() {
    let store = Arc::new(MemorySpend::default());
    let budget = budget(&store, caps());
    budget
        .charge(Some("42"), 140, moment())
        .expect("a charge is written")
        .await
        .unwrap();
    budget
        .charge(None, 6, moment())
        .expect("a charge is written")
        .await
        .unwrap();
    let counters = store.counters();
    assert_eq!(counters["chat.month.2026-03"], 146);
    assert_eq!(counters["chat.day.2026-03-09"], 146);
    assert_eq!(counters["chat.user.42.month.2026-03"], 140);
    assert_eq!(counters["chat.user.42.day.2026-03-09"], 140);
    assert_eq!(counters.len(), 4);
}

#[tokio::test]
async fn only_capped_periods_are_counted_and_nothing_is_written_for_free_answers() {
    let store = Arc::new(MemorySpend::default());
    let only_day = Caps {
        day: Some(100),
        ..Caps::default()
    };
    let budget = budget(&store, only_day);
    assert!(budget.charge(Some("42"), 0, moment()).is_none());
    budget
        .charge(Some("42"), 5, moment())
        .unwrap()
        .await
        .unwrap();
    assert_eq!(
        store.counters().into_iter().collect::<Vec<_>>(),
        [("chat.day.2026-03-09".to_owned(), 5)]
    );
}

#[tokio::test]
async fn any_reached_cap_spends_the_budget() {
    for (key, user) in [
        ("chat.month.2026-03", None),
        ("chat.day.2026-03-09", None),
        ("chat.user.42.month.2026-03", Some("42")),
        ("chat.user.42.day.2026-03-09", Some("42")),
    ] {
        let store = Arc::new(MemorySpend::default());
        let budget = budget(&store, caps());
        assert!(!budget.is_spent(user, moment()).await, "{key} starts empty");
        store.set(key, 1_000_000);
        assert!(budget.is_spent(user, moment()).await, "{key} is reached");
    }
}

#[tokio::test]
async fn one_user_at_the_cap_does_not_stop_another() {
    let store = Arc::new(MemorySpend::default());
    let budget = budget(&store, caps());
    store.set("chat.user.42.day.2026-03-09", 10_000);
    assert!(budget.is_spent(Some("42"), moment()).await);
    assert!(!budget.is_spent(Some("43"), moment()).await);
    assert!(!budget.is_spent(None, moment()).await);
}

#[tokio::test]
async fn a_new_day_and_a_new_month_start_from_zero() {
    let store = Arc::new(MemorySpend::default());
    let budget = budget(&store, caps());
    store.set("chat.day.2026-03-09", 200_000);
    assert!(budget.is_spent(None, moment()).await);
    let next_day = moment() + chrono::Duration::seconds(1);
    assert!(!budget.is_spent(None, next_day).await);
    store.set("chat.month.2026-03", 1_000_000);
    assert!(budget.is_spent(None, next_day).await);
    let next_month = Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap();
    assert!(!budget.is_spent(None, next_month).await);
}

#[tokio::test]
async fn an_unreachable_store_counts_as_spent() {
    let store = Arc::new(MemorySpend::default());
    let budget = budget(&store, caps());
    store.go_down();
    assert!(budget.is_spent(None, moment()).await);
}

#[test]
fn a_user_key_outside_the_counter_alphabet_is_hashed() {
    assert_eq!(user_token("12345"), "12345");
    assert_eq!(user_token("soundcloud_users-7"), "soundcloud_users-7");
    let hashed = user_token("soundcloud:users:7");
    assert_eq!(hashed.len(), 33);
    assert!(hashed.starts_with('h') && hashed != user_token("soundcloud:users:8"));
    assert_eq!(user_token("").len(), 33);
}

#[test]
fn caps_come_from_dollars() {
    let caps = Caps::from(CapSettings {
        month_usd: Some(1.5),
        day_usd: None,
        user_month_usd: None,
        user_day_usd: Some(0.02),
    });
    assert_eq!(caps.month, Some(1_500_000));
    assert_eq!(caps.user_day, Some(20_000));
    assert!(caps.is_set() && !Caps::default().is_set());
}
