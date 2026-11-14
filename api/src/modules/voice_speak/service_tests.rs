use std::time::Instant;

use uuid::{NoContext, Timestamp};

use super::*;
use crate::modules::voice_speak::testing::{
    Answer, CLIP, QUICK, Speaking, reply, speaking, uncached,
};

const USER: &str = "98765";
const KEPT_KEY: &str = "loli/409e921e77f2c3acad538a6a236e7dbdff0cba38f0151f3a4e17fca05a6e7640.ogg";

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

fn ticket_from(ago: Duration) -> Uuid {
    let at = SystemTime::now() - ago;
    let since = at.duration_since(UNIX_EPOCH).expect("after the epoch");
    Uuid::new_v7(Timestamp::from_unix(
        NoContext,
        since.as_secs(),
        since.subsec_nanos(),
    ))
}

#[tokio::test]
async fn a_kept_line_found_in_the_phrase_cache_answers_the_clip_without_a_ticket() {
    let speaking = speaking(Answer::Clip, 4);
    let cached = line("Держи.", true);
    speaking.phrases.put(&cached.phrase_key(), CLIP);

    let said = speaking.live().say(USER, cached).await;

    assert_eq!(said, Ok(Said::Cached(clip())));
    assert!(speaking.lane.asked().is_empty());
    assert!(speaking.replies.subjects().is_empty());
}

#[tokio::test]
async fn a_plain_line_is_asked_with_the_reply_subject_of_its_user_and_ticket() {
    let speaking = speaking(Answer::Silent, 4);
    let plain = line("Привет.", false);
    speaking.phrases.put(&plain.phrase_key(), CLIP);

    let ticket = ticket(&speaking, plain).await;

    let asked = speaking.lane.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].reply_to, format!("speak.reply.{USER}.{ticket}"));
    assert_eq!(asked[0].ticket, ticket);
    assert_eq!(ticket.get_version_num(), 7);
    assert_eq!(
        asked[0].request,
        SayRequest {
            voice: "loli".to_owned(),
            text: "Привет.".to_owned(),
        }
    );
    assert_eq!(
        asked[0].deadline,
        issued(ticket).expect("a v7 ticket") + Duration::from_secs(SAY_WINDOW_SECONDS)
    );
    assert_eq!(speaking.phrases.reads(), 0);
    assert!(speaking.replies.subjects().is_empty());
}

#[tokio::test]
async fn a_kept_line_looks_the_cache_up_and_publishes_its_key_under_its_ticket() {
    let speaking = speaking(Answer::Silent, 4);

    let ticket = ticket(&speaking, line("Держи.", true)).await;

    assert_eq!(speaking.phrases.reads(), 1);
    assert_eq!(
        speaking.replies.get(&format!("speak.keep.{USER}.{ticket}")),
        Some(Bytes::from_static(KEPT_KEY.as_bytes()))
    );
    assert_eq!(speaking.lane.asked().len(), 1);
}

#[tokio::test]
async fn a_full_lane_or_a_refused_publish_is_busy_and_gets_no_ticket() {
    let full = speaking(Answer::Clip, 2);
    full.lane.fill(2);
    let refused = speaking(Answer::Clip, 2);
    refused.lane.refuse();
    let unkept = speaking(Answer::Clip, 2);
    unkept.replies.break_down();

    assert_eq!(full.live().say(USER, line("Раз.", false)).await, Err(Busy));
    assert_eq!(
        refused.live().say(USER, line("Два.", false)).await,
        Err(Busy)
    );
    assert_eq!(unkept.live().say(USER, line("Три.", true)).await, Err(Busy));
    assert!(full.lane.asked().is_empty());
    assert!(refused.lane.asked().is_empty());
    assert!(unkept.lane.asked().is_empty());
    full.lane.fill(1);
    ticket(&full, line("Раз.", false)).await;
}

#[tokio::test]
async fn the_poll_answers_the_clip_of_an_ok_reply() {
    let speaking = speaking(Answer::Clip, 4);

    let ticket = ticket(&speaking, line("Привет.", false)).await;

    assert_eq!(
        speaking.live().fetch(USER, ticket).await,
        Fetched::Clip(clip())
    );
    assert!(speaking.phrases.names().is_empty());
}

#[tokio::test]
async fn the_first_poll_of_a_kept_line_stores_the_clip_under_its_phrase_key() {
    let speaking = speaking(Answer::Clip, 4);
    let kept = line("Держи.", true);

    let ticket = ticket(&speaking, kept.clone()).await;
    let fetched = speaking.live().fetch(USER, ticket).await;

    assert_eq!(fetched, Fetched::Clip(clip()));
    assert_eq!(speaking.phrases.names(), [KEPT_KEY]);
    assert_eq!(speaking.phrases.get(KEPT_KEY), Some(clip()));
    assert_eq!(
        speaking.live().say(USER, kept).await,
        Ok(Said::Cached(clip()))
    );
    assert_eq!(speaking.lane.asked().len(), 1);
}

#[tokio::test]
async fn a_failed_reply_or_a_clip_that_is_not_ogg_fails_the_poll_and_keeps_nothing() {
    for answer in [
        Answer::Refuse("internal"),
        Answer::Refuse("invalid_request"),
        Answer::Refuse("expired"),
        Answer::Malformed,
        Answer::Encoded("%%% not base64"),
        Answer::Encoded(""),
        Answer::Encoded("UklGRiQAAABXQVZF"),
    ] {
        let speaking = speaking(answer, 4);

        let ticket = ticket(&speaking, line("Не выйдет.", true)).await;

        assert_eq!(speaking.live().fetch(USER, ticket).await, Fetched::Failed);
        assert!(speaking.phrases.names().is_empty());
    }
}

#[tokio::test]
async fn a_ticket_past_its_window_without_a_reply_is_expired_and_a_fresh_one_is_pending() {
    let speaking = speaking(Answer::Silent, 4);
    let late = ticket_from(Duration::from_secs(SAY_WINDOW_SECONDS + 6));
    let fresh = ticket_from(Duration::from_secs(SAY_WINDOW_SECONDS - 6));

    let started = Instant::now();
    let expired = speaking.live().fetch(USER, late).await;
    let expired_after = started.elapsed();
    let started = Instant::now();
    let pending = speaking.live().fetch(USER, fresh).await;
    let held_for = started.elapsed();

    assert_eq!(expired, Fetched::Expired);
    assert!(expired_after < QUICK.length, "{expired_after:?}");
    assert_eq!(pending, Fetched::Pending);
    assert!(held_for >= QUICK.length, "{held_for:?}");
}

#[tokio::test]
async fn a_late_reply_still_answers_the_poll() {
    let speaking = speaking(Answer::Silent, 4);
    let late = ticket_from(Duration::from_secs(SAY_WINDOW_SECONDS + 60));
    let answered = reply(Answer::Clip).expect("an ok reply").to_string();
    speaking
        .replies
        .put(&format!("speak.reply.{USER}.{late}"), answered.as_bytes());

    assert_eq!(
        speaking.live().fetch(USER, late).await,
        Fetched::Clip(clip())
    );
}

#[tokio::test]
async fn a_reply_that_arrives_during_the_hold_is_answered_at_once() {
    let speaking = speaking(Answer::Silent, 4);
    let ticket = ticket(&speaking, line("Сейчас.", false)).await;
    let replies = speaking.replies.clone();
    let answered = reply(Answer::Clip).expect("an ok reply").to_string();
    let worker = tokio::spawn(async move {
        tokio::time::sleep(QUICK.length / 4).await;
        replies.put(&format!("speak.reply.{USER}.{ticket}"), answered.as_bytes());
    });

    let started = Instant::now();
    let fetched = speaking.live().fetch(USER, ticket).await;

    assert_eq!(fetched, Fetched::Clip(clip()));
    assert!(started.elapsed() < QUICK.length);
    worker.await.expect("the reply is written");
}

#[tokio::test]
async fn a_poll_never_finds_the_reply_of_another_user() {
    let speaking = speaking(Answer::Clip, 4);
    let ticket = ticket(&speaking, line("Секрет.", false)).await;

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
async fn without_a_phrase_cache_a_kept_line_is_neither_looked_up_nor_kept() {
    let speaking = uncached(Answer::Clip, 4);
    let kept = line("Держи.", true);
    speaking.phrases.put(&kept.phrase_key(), CLIP);

    let first = ticket(&speaking, kept.clone()).await;
    let second = ticket(&speaking, kept).await;

    assert_eq!(
        speaking.live().fetch(USER, first).await,
        Fetched::Clip(clip())
    );
    assert_eq!(
        speaking.live().fetch(USER, second).await,
        Fetched::Clip(clip())
    );
    assert_eq!(speaking.lane.asked().len(), 2);
    assert_eq!(speaking.phrases.reads(), 0);
    assert_eq!(speaking.phrases.names().len(), 1);
    assert!(
        speaking
            .replies
            .subjects()
            .iter()
            .all(|subject| subject.starts_with("speak.reply."))
    );
}

#[tokio::test]
async fn a_broken_phrase_cache_is_a_miss_and_its_failed_write_still_answers_the_clip() {
    let speaking = speaking(Answer::Clip, 4);
    speaking.phrases.break_down();

    let ticket = ticket(&speaking, line("Всё равно.", true)).await;

    assert_eq!(speaking.phrases.reads(), 1);
    assert_eq!(
        speaking.live().fetch(USER, ticket).await,
        Fetched::Clip(clip())
    );
    assert!(speaking.phrases.names().is_empty());
}
