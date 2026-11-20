use chrono::{SubsecRound, TimeZone};
use serde_json::json;
use sqlx::PgPool;

use super::*;
use crate::modules::history::stats::StatsPeriod;

const USER: &str = "soundcloud:users:7";

fn play(track: &str, played_at: Option<DateTime<Utc>>) -> RecordHistoryDto {
    RecordHistoryDto {
        sc_track_id: track.to_owned(),
        title: format!("Song {track}"),
        artist_name: "Artist".to_owned(),
        artist_urn: None,
        artwork_url: None,
        duration: 200_000,
        played_at,
    }
}

fn hours_ago(hours: i64) -> DateTime<Utc> {
    (Utc::now() - Duration::hours(hours)).trunc_subsecs(0)
}

async fn stored(pool: &PgPool) -> anyhow::Result<Vec<NaiveDateTime>> {
    Ok(
        sqlx::query_scalar("SELECT played_at FROM listening_history ORDER BY played_at")
            .fetch_all(pool)
            .await?,
    )
}

#[test]
fn a_client_time_is_kept_inside_the_bounds_and_now_is_used_outside() {
    let now = Utc
        .with_ymd_and_hms(2026, 11, 20, 12, 0, 0)
        .single()
        .expect("time");
    let kept = now - Duration::days(29);
    assert_eq!(played_at(Some(kept), now), kept.naive_utc());
    assert_eq!(
        played_at(Some(now + Duration::seconds(90)), now),
        now.naive_utc()
    );
    assert_eq!(
        played_at(Some(now + Duration::minutes(10)), now),
        now.naive_utc()
    );
    assert_eq!(
        played_at(Some(now - Duration::days(31)), now),
        now.naive_utc()
    );
    assert_eq!(played_at(None, now), now.naive_utc());
}

#[test]
fn the_desktop_iso_time_is_read_and_an_old_client_sends_none() -> anyhow::Result<()> {
    let body =
        json!({"scTrackId": "61", "title": "Song", "artistName": "Artist", "duration": 1000});
    let old: RecordHistoryDto = serde_json::from_value(body.clone())?;
    assert_eq!(old.played_at, None);
    let mut queued = body;
    queued["playedAt"] = json!("2026-11-20T09:15:30.250Z");
    let queued: RecordHistoryDto = serde_json::from_value(queued)?;
    let expected = Utc
        .with_ymd_and_hms(2026, 11, 20, 9, 15, 30)
        .single()
        .expect("time")
        + Duration::milliseconds(250);
    assert_eq!(queued.played_at, Some(expected));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_queued_play_keeps_its_time_and_a_resend_is_the_same_row(
    pool: PgPool,
) -> anyhow::Result<()> {
    let history = HistoryService::new(pool.clone());
    let at = hours_ago(3);
    let first = history.record(USER, &play("61", Some(at))).await?;
    let resent = history.record(USER, &play("61", Some(at))).await?;
    assert_eq!(first, resent);
    let replayed = history
        .record(USER, &play("61", Some(at + Duration::seconds(30))))
        .await?;
    assert_eq!(first, replayed);
    let later = history
        .record(USER, &play("61", Some(at + Duration::minutes(4))))
        .await?;
    assert_ne!(first, later);
    assert_eq!(
        stored(&pool).await?,
        vec![at.naive_utc(), (at + Duration::minutes(4)).naive_utc()]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn two_sends_of_one_play_at_once_store_one_row(pool: PgPool) -> anyhow::Result<()> {
    let history = HistoryService::new(pool.clone());
    let dto = play("62", Some(hours_ago(1)));
    let (a, b) = tokio::join!(history.record(USER, &dto), history.record(USER, &dto));
    assert_eq!(a?, b?);
    assert_eq!(stored(&pool).await?.len(), 1);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn stats_place_a_queued_play_on_the_day_it_was_played(pool: PgPool) -> anyhow::Result<()> {
    let history = HistoryService::new(pool);
    let at = hours_ago(72);
    history.record(USER, &play("63", Some(at))).await?;
    history.record(USER, &play("64", None)).await?;
    let stats = history
        .stats(USER, StatsPeriod::Week, 0, Utc::now().naive_utc())
        .await?;
    let days: Vec<(chrono::NaiveDate, i64)> =
        stats.timeline.iter().map(|p| (p.date, p.plays)).collect();
    assert_eq!(
        days,
        vec![(at.date_naive(), 1), (Utc::now().date_naive(), 1)]
    );
    Ok(())
}
