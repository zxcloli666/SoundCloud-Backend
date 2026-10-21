use std::process::Command;
use std::time::Duration;

use axum::http::StatusCode;
use uuid::Uuid;

use super::test_helpers::{Answer, caller, poll, sync, warm, with_max_parked, without_postgres};
use crate::modules::chat::ChatState;
use crate::modules::chat::sync::wire::RETRY_AFTER_MS;
use crate::modules::chat::sync::{bump, changes};

const USERS: usize = 10_000;
const DEVICES_PER_USER: usize = 3;
const BATCH: usize = 300;
const BUSY_PAUSE: Duration = Duration::from_millis(100);
const RSS_LIMIT_BYTES: u64 = 1_500 * 1024 * 1024;

fn rss_bytes() -> u64 {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .expect("an rss in KiB")
        * 1024
}

fn told_to_wait(answer: &Answer) -> bool {
    answer.headers.contains_key(RETRY_AFTER_MS)
}

async fn until_parked(chat: &ChatState, parked: usize) {
    for _ in 0..3_000 {
        if chat.hub.parked() + parked / 50 >= parked {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("only {} of {parked} polls parked", chat.hub.parked());
}

async fn bumped(chat: &ChatState, user: &str) -> u64 {
    while bump::devices(chat, user).await.is_err() {
        tokio::time::sleep(BUSY_PAUSE).await;
    }
    loop {
        match changes::read(chat, user, 0).await {
            Ok(changes) => return changes.versions.version,
            Err(_) => tokio::time::sleep(BUSY_PAUSE).await,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "soak: 30k parked polls against a local Redis"]
async fn thirty_thousand_parked_polls_stay_under_one_and_a_half_gigabytes() {
    let chat = with_max_parked(without_postgres(), USERS * DEVICES_PER_USER + 10);
    let users: Vec<String> = (0..USERS)
        .map(|n| format!("soak:{}:{n}", Uuid::now_v7()))
        .collect();
    let names: Vec<&str> = users.iter().map(String::as_str).collect();
    let incarnation = warm(&chat, &names);
    let before = rss_bytes();

    let mut polls = Vec::with_capacity(USERS * DEVICES_PER_USER);
    for batch in users.chunks(BATCH / DEVICES_PER_USER) {
        for user in batch {
            let pos = bumped(&chat, user).await;
            for _ in 0..DEVICES_PER_USER {
                let chat = chat.clone();
                let caller = caller(user, Uuid::now_v7());
                polls.push(tokio::spawn(async move {
                    loop {
                        let answer =
                            sync(&chat, caller.clone(), poll(pos, incarnation, 20_000)).await;
                        if answer.status == StatusCode::SERVICE_UNAVAILABLE {
                            tokio::time::sleep(BUSY_PAUSE).await;
                            continue;
                        }
                        if answer.status != StatusCode::NO_CONTENT || told_to_wait(&answer) {
                            return answer;
                        }
                    }
                }));
            }
        }
        until_parked(&chat, polls.len()).await;
    }
    assert!(chat.hub.parked() > USERS * DEVICES_PER_USER - BATCH);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let parked_rss = rss_bytes();
    println!(
        "parked {} polls: rss {} MiB, grew {} MiB",
        chat.hub.parked(),
        parked_rss / 1024 / 1024,
        parked_rss.saturating_sub(before) / 1024 / 1024
    );
    assert!(parked_rss < RSS_LIMIT_BYTES);

    chat.shutdown.cancel();
    for handle in polls {
        let answer = handle.await.expect("a poll");
        assert_eq!(answer.status, StatusCode::NO_CONTENT);
        assert!(told_to_wait(&answer));
    }
    assert_eq!(chat.hub.parked(), 0);
}
