use std::time::{Duration, Instant};

use axum::http::StatusCode;
use redis::AsyncCommands;
use uuid::Uuid;

use super::test_helpers::{
    caller, poll, position, redis_pool, redis_url, settle, sync, user, warm, with_max_parked,
    without_postgres,
};
use crate::modules::chat::sync::bump;
use crate::modules::chat::sync::changes::{self, Change};
use crate::modules::chat::sync::prefs::{self, Prefs};
use crate::modules::chat::sync::reply::LATER_MAX_MS;
use crate::modules::chat::sync::wire::RETRY_AFTER_MS;
use crate::modules::chat::{presence, rt_keys};

fn retry_ms(answer: &super::test_helpers::Answer) -> Option<u64> {
    answer
        .headers
        .get(RETRY_AFTER_MS)
        .and_then(|value| value.to_str().ok()?.parse().ok())
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn an_idle_poll_runs_no_postgres_query() {
    let chat = without_postgres();
    let user = user();
    let incarnation = warm(&chat, &[&user]);
    let sharing = Prefs {
        presence_share: true,
        ..Prefs::default()
    };
    prefs::changed(&chat, &user, sharing).await.expect("prefs");
    chat.hub.prefs.expire(&user);
    bump::devices(&chat, &user).await.expect("a bump");
    let pos = position(&chat, &user).await;
    let device = Uuid::now_v7();

    let started = Instant::now();
    let held = sync(&chat, caller(&user, device), poll(pos, incarnation, 300)).await;
    assert_eq!(held.status, StatusCode::NO_CONTENT, "{}", held.body);
    assert!(started.elapsed() >= Duration::from_millis(250));
    assert_eq!(retry_ms(&held), None);

    let short = sync(&chat, caller(&user, device), poll(pos, incarnation, 0)).await;
    assert_eq!(short.status, StatusCode::NO_CONTENT, "{}", short.body);

    let mut conn = redis_pool(&redis_url()).get().await.expect("redis");
    let seen: Option<f64> = conn.zscore(rt_keys::SEEN, &user).await.expect("a score");
    assert!(seen.is_some(), "presence is marked from the cached opt-in");
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn turning_presence_off_drops_the_user_from_the_seen_set() {
    let chat = without_postgres();
    let user = user();
    let sharing = Prefs {
        presence_share: true,
        ..Prefs::default()
    };
    prefs::changed(&chat, &user, sharing).await.expect("prefs");
    presence::mark_seen(&chat, &user).await.expect("seen");
    prefs::changed(&chat, &user, Prefs::default())
        .await
        .expect("prefs");

    let mut conn = redis_pool(&redis_url()).get().await.expect("redis");
    let seen: Option<f64> = conn.zscore(rt_keys::SEEN, &user).await.expect("a score");
    assert_eq!(seen, None);
    presence::mark_seen(&chat, &user).await.expect("seen");
    let seen: Option<f64> = conn.zscore(rt_keys::SEEN, &user).await.expect("a score");
    assert_eq!(seen, None);
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn parked_polls_end_with_a_jittered_retry_on_shutdown() {
    let chat = without_postgres();
    let user = user();
    let incarnation = warm(&chat, &[&user]);
    bump::devices(&chat, &user).await.expect("a bump");
    let pos = position(&chat, &user).await;

    let mut polls = Vec::new();
    for _ in 0..20 {
        let chat = chat.clone();
        let caller = caller(&user, Uuid::now_v7());
        polls.push(tokio::spawn(async move {
            sync(&chat, caller, poll(pos, incarnation, 20_000)).await
        }));
    }
    settle(&chat, 20).await;
    let started = Instant::now();
    chat.shutdown.cancel();
    for handle in polls {
        let answer = handle.await.expect("a poll");
        assert_eq!(answer.status, StatusCode::NO_CONTENT);
        assert!(retry_ms(&answer).expect("a retry header") <= LATER_MAX_MS);
    }
    assert!(started.elapsed() < Duration::from_secs(2));

    let late = sync(
        &chat,
        caller(&user, Uuid::now_v7()),
        poll(pos, incarnation, 20_000),
    )
    .await;
    assert_eq!(late.status, StatusCode::NO_CONTENT);
    assert!(retry_ms(&late).is_some());
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn the_newest_poll_of_a_device_ends_the_older_one() {
    let chat = without_postgres();
    let user = user();
    let incarnation = warm(&chat, &[&user]);
    bump::devices(&chat, &user).await.expect("a bump");
    let pos = position(&chat, &user).await;
    let device = Uuid::now_v7();

    let older = {
        let chat = chat.clone();
        let caller = caller(&user, device);
        tokio::spawn(async move { sync(&chat, caller, poll(pos, incarnation, 20_000)).await })
    };
    settle(&chat, 1).await;
    let newer = {
        let chat = chat.clone();
        let caller = caller(&user, device);
        tokio::spawn(async move { sync(&chat, caller, poll(pos, incarnation, 20_000)).await })
    };
    let older = tokio::time::timeout(Duration::from_secs(2), older)
        .await
        .expect("the older poll ends")
        .expect("a poll");
    assert_eq!(older.status, StatusCode::NO_CONTENT);
    assert_eq!(retry_ms(&older), None);
    assert!(!newer.is_finished());
    chat.shutdown.cancel();
    newer.await.expect("a poll");
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn polls_over_the_parked_cap_are_turned_away() {
    let chat = with_max_parked(without_postgres(), 2);
    let user = user();
    let incarnation = warm(&chat, &[&user]);
    bump::devices(&chat, &user).await.expect("a bump");
    let pos = position(&chat, &user).await;

    let mut polls = Vec::new();
    for _ in 0..3 {
        let chat = chat.clone();
        let caller = caller(&user, Uuid::now_v7());
        polls.push(tokio::spawn(async move {
            sync(&chat, caller, poll(pos, incarnation, 20_000)).await
        }));
    }
    let (turned, _, rest) =
        tokio::time::timeout(Duration::from_secs(3), futures::future::select_all(polls))
            .await
            .expect("one poll is turned away");
    let turned = turned.expect("a poll");
    assert_eq!(turned.status, StatusCode::NO_CONTENT);
    assert!(retry_ms(&turned).is_some());
    assert!((1..=2).contains(&chat.hub.parked()));
    chat.shutdown.cancel();
    for handle in rest {
        handle.await.expect("a poll");
    }
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_client_that_never_waits_is_slowed_after_twenty_answers() {
    let chat = without_postgres();
    let user = user();
    let incarnation = warm(&chat, &[&user]);
    bump::devices(&chat, &user).await.expect("a bump");
    let pos = position(&chat, &user).await;
    let device = Uuid::now_v7();

    let started = Instant::now();
    for _ in 0..20 {
        let answer = sync(&chat, caller(&user, device), poll(pos, incarnation, 0)).await;
        assert_eq!(answer.status, StatusCode::NO_CONTENT);
    }
    assert!(started.elapsed() < Duration::from_secs(1));
    let slowed = Instant::now();
    sync(&chat, caller(&user, device), poll(pos, incarnation, 0)).await;
    assert!(slowed.elapsed() >= Duration::from_millis(950));
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_bump_moves_the_version_keeps_256_changes_and_raises_the_floor() {
    let chat = without_postgres();
    let user = user();
    let conversation = Uuid::now_v7();
    bump::conversation(&chat, conversation, 7, std::slice::from_ref(&user))
        .await
        .expect("a bump");
    let first = changes::read(&chat, &user, 0).await.expect("versions");
    assert!(!first.versions.fresh);
    assert!(first.versions.version > first.versions.floor);

    let mut conn = redis_pool(&redis_url()).get().await.expect("redis");
    let seq: i64 = conn
        .get(rt_keys::conversation_seq(conversation))
        .await
        .expect("a seq");
    assert_eq!(seq, 7);
    bump::conversation(&chat, conversation, 3, &[])
        .await
        .expect("a bump");
    let seq: i64 = conn
        .get(rt_keys::conversation_seq(conversation))
        .await
        .expect("a seq");
    assert_eq!(seq, 7);

    let pos = first.versions.version;
    for _ in 0..300 {
        bump::conversation(&chat, Uuid::now_v7(), 1, std::slice::from_ref(&user))
            .await
            .expect("a bump");
    }
    let kept: usize = conn.zcard(rt_keys::deltas(&user)).await.expect("a count");
    assert_eq!(kept, 256);
    let later = changes::read(&chat, &user, pos).await.expect("versions");
    assert!(later.versions.floor > pos);
    assert!(later.deltas.is_empty());
    let current = changes::read(&chat, &user, later.versions.floor)
        .await
        .expect("versions");
    assert_eq!(current.deltas.len(), 256);
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_missing_version_is_created_fresh_and_reported_once() {
    let chat = without_postgres();
    let user = user();
    let first = changes::read(&chat, &user, 0).await.expect("versions");
    assert!(first.versions.fresh);
    assert_eq!(first.versions.version, first.versions.floor);
    let second = changes::read(&chat, &user, first.versions.version)
        .await
        .expect("versions");
    assert!(!second.versions.fresh);
    assert_eq!(
        second.versions,
        crate::modules::chat::sync::changes::Versions {
            fresh: false,
            ..first.versions
        }
    );
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn gone_messages_are_split_into_bounded_entries() {
    let chat = without_postgres();
    let user = user();
    let conversation = Uuid::now_v7();
    let pos = changes::read(&chat, &user, 0)
        .await
        .expect("versions")
        .versions
        .version;
    let seqs: Vec<i64> = (1..=150).collect();
    bump::messages_gone(&chat, conversation, &seqs, std::slice::from_ref(&user))
        .await
        .expect("a bump");
    let changed = changes::read(&chat, &user, pos).await.expect("versions");
    let mut found: Vec<i64> = Vec::new();
    for delta in &changed.deltas {
        let Change::GoneMessages(id, seqs) = &delta.change else {
            panic!("only gone messages were bumped");
        };
        assert_eq!(*id, conversation);
        assert!(seqs.len() <= bump::GONE_SEQS_PER_ENTRY);
        found.extend(seqs);
    }
    found.sort_unstable();
    assert_eq!(found, seqs);
}
