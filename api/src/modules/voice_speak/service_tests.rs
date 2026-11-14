use std::time::Instant;

use backend_contracts::pipeline::{SAY_WINDOW_SECONDS, SPEAK_SAY};
use llm_ladder::WorkerError;
use serde_json::json;

use super::*;
use crate::modules::voice_speak::testing::{
    Answer, CLIP, QUICK, Speaking, held, speaking, uncached,
};

const USER: &str = "98765";

fn line(text: &str, keep: bool) -> Line {
    Line {
        voice: "loli",
        text: text.to_owned(),
        keep,
    }
}

async fn ticket(speaking: &Speaking, line: Line) -> Uuid {
    match speaking.live().say(USER, line).await {
        Ok(Said::Ticket(ticket)) => ticket,
        other => panic!("a ticket, not {other:?}"),
    }
}

fn clip() -> Bytes {
    Bytes::from_static(CLIP)
}

#[tokio::test]
async fn a_kept_line_found_in_the_phrase_cache_answers_the_clip_without_a_ticket() {
    let speaking = speaking(Answer::Clip, 4);
    let cached = line("Привет.", true);
    speaking.phrases.put(&cached.phrase_key(), CLIP);

    let said = speaking.live().say(USER, cached).await;

    assert_eq!(said, Ok(Said::Cached(clip())));
    assert!(speaking.voice.asked().is_empty());
    assert!(speaking.clips.names().is_empty());
}

#[tokio::test]
async fn a_line_not_kept_never_reads_the_phrase_cache_and_goes_to_the_worker() {
    let speaking = speaking(Answer::Clip, 4);
    let passing = line("Привет.", false);
    speaking.phrases.put(&passing.phrase_key(), CLIP);

    ticket(&speaking, passing).await;
    speaking.settled().await;

    assert_eq!(speaking.phrases.reads(), 0);
    assert_eq!(speaking.voice.asked().len(), 1);
}

#[tokio::test]
async fn a_new_line_answers_a_ticket_and_the_decoded_clip_waits_for_the_poll() {
    let speaking = speaking(Answer::Clip, 4);

    let ticket = ticket(&speaking, line("Привет.", false)).await;
    speaking.settled().await;

    let asked = speaking.voice.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].subject, SPEAK_SAY);
    assert_eq!(asked[0].timeout, Duration::from_secs(SAY_WINDOW_SECONDS));
    assert_eq!(
        asked[0].request,
        json!({"voice": "loli", "text": "Привет."})
    );
    assert_eq!(ticket.get_version_num(), 7);
    assert_eq!(
        speaking.clips.get(&format!("{USER}.{ticket}")),
        Some(clip())
    );
    assert_eq!(speaking.clips.names().len(), 1);
    assert_eq!(
        speaking.live().fetch(USER, ticket).await,
        Fetched::Clip(clip())
    );
}

#[tokio::test]
async fn a_kept_line_stores_the_same_clip_under_its_voice_and_hash_and_a_passing_one_does_not() {
    let speaking = speaking(Answer::Clip, 4);
    let kept = line("Держи.", true);
    let passing = line("Мимо.", false);

    ticket(&speaking, kept.clone()).await;
    ticket(&speaking, passing).await;
    speaking.settled().await;

    assert_eq!(
        speaking.phrases.names(),
        ["loli/409e921e77f2c3acad538a6a236e7dbdff0cba38f0151f3a4e17fca05a6e7640.ogg"]
    );
    assert_eq!(speaking.phrases.get(&kept.phrase_key()), Some(clip()));
    let again = speaking.live().say(USER, kept).await;
    assert_eq!(again, Ok(Said::Cached(clip())));
    assert_eq!(speaking.voice.asked().len(), 2);
}

#[tokio::test]
async fn a_failed_reply_or_a_clip_that_is_not_ogg_leaves_the_empty_mark_and_keeps_nothing() {
    for answer in [
        Answer::Refuse(WorkerError::Rejected),
        Answer::Refuse(WorkerError::Invalid),
        Answer::Refuse(WorkerError::Timeout),
        Answer::Refuse(WorkerError::Unreachable),
        Answer::Malformed,
        Answer::Encoded("%%% not base64"),
        Answer::Encoded(""),
        Answer::Encoded("UklGRiQAAABXQVZF"),
    ] {
        let speaking = speaking(answer, 4);

        let ticket = ticket(&speaking, line("Не выйдет.", true)).await;
        speaking.settled().await;

        assert_eq!(
            speaking.clips.get(&format!("{USER}.{ticket}")),
            Some(Bytes::new())
        );
        assert_eq!(speaking.live().fetch(USER, ticket).await, Fetched::Failed);
        assert!(speaking.phrases.names().is_empty());
    }
}

#[tokio::test]
async fn over_the_cap_a_line_is_busy_and_gets_no_ticket() {
    let speaking = held(Answer::Clip, 2);

    ticket(&speaking, line("Раз.", false)).await;
    ticket(&speaking, line("Два.", false)).await;
    let third = speaking.live().say(USER, line("Три.", false)).await;
    speaking.voice.open();
    speaking.settled().await;

    assert_eq!(third, Err(Busy));
    let asked: Vec<String> = speaking
        .voice
        .asked()
        .iter()
        .map(|asked| asked.request["text"].to_string())
        .collect();
    assert_eq!(asked.len(), 2);
    assert!(!asked.iter().any(|text| text.contains("Три")));
    ticket(&speaking, line("Три.", false)).await;
}

#[tokio::test]
async fn the_poll_answers_the_clip_the_failure_or_nothing_after_the_hold() {
    let speaking = speaking(Answer::Clip, 4);
    let (voiced, failed, silent) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    speaking.clips.put(&format!("{USER}.{voiced}"), CLIP);
    speaking.clips.put(&format!("{USER}.{failed}"), b"");

    let started = Instant::now();
    let nothing = speaking.live().fetch(USER, silent).await;
    let held_for = started.elapsed();

    assert_eq!(
        speaking.live().fetch(USER, voiced).await,
        Fetched::Clip(clip())
    );
    assert_eq!(speaking.live().fetch(USER, failed).await, Fetched::Failed);
    assert_eq!(nothing, Fetched::Pending);
    assert!(held_for >= QUICK.length, "{held_for:?}");
}

#[tokio::test]
async fn a_clip_that_arrives_during_the_hold_is_answered_at_once() {
    let speaking = speaking(Answer::Clip, 4);
    let ticket = Uuid::now_v7();
    let clips = speaking.clips.clone();
    let writer = tokio::spawn(async move {
        tokio::time::sleep(QUICK.length / 4).await;
        clips.put(&format!("{USER}.{ticket}"), CLIP);
    });

    let started = Instant::now();
    let fetched = speaking.live().fetch(USER, ticket).await;

    assert_eq!(fetched, Fetched::Clip(clip()));
    assert!(started.elapsed() < QUICK.length);
    writer.await.expect("the clip is written");
}

#[tokio::test]
async fn a_poll_never_finds_the_clip_of_another_user() {
    let speaking = speaking(Answer::Clip, 4);
    let ticket = ticket(&speaking, line("Секрет.", false)).await;
    speaking.settled().await;

    assert_eq!(
        speaking.live().fetch("11111", ticket).await,
        Fetched::Pending
    );
    assert_eq!(
        speaking.live().fetch(USER, ticket).await,
        Fetched::Clip(clip())
    );
}

#[tokio::test]
async fn a_broken_phrase_cache_is_a_miss_and_the_line_is_still_voiced() {
    let speaking = speaking(Answer::Clip, 4);
    speaking.phrases.break_down();

    let ticket = ticket(&speaking, line("Всё равно.", true)).await;
    speaking.settled().await;

    assert_eq!(speaking.phrases.reads(), 1);
    assert_eq!(
        speaking.live().fetch(USER, ticket).await,
        Fetched::Clip(clip())
    );
    assert_eq!(speaking.voice.asked().len(), 1);
}

#[tokio::test]
async fn without_a_phrase_cache_a_kept_line_goes_to_the_worker_and_nothing_is_kept() {
    let speaking = uncached(Answer::Clip, 4);
    let kept = line("Держи.", true);

    let first = ticket(&speaking, kept.clone()).await;
    let second = ticket(&speaking, kept).await;
    speaking.settled().await;

    assert_eq!(speaking.voice.asked().len(), 2);
    assert_eq!(
        speaking.live().fetch(USER, first).await,
        Fetched::Clip(clip())
    );
    assert_eq!(
        speaking.live().fetch(USER, second).await,
        Fetched::Clip(clip())
    );
    assert_eq!(speaking.clips.names().len(), 2);
    assert_eq!(speaking.phrases.reads(), 0);
    assert!(speaking.phrases.names().is_empty());
}
