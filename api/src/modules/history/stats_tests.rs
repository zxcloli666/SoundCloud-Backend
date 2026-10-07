use chrono::{NaiveDate, NaiveDateTime};
use sqlx::PgPool;

use super::*;

const USER: &str = "soundcloud:users:7";

fn at(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").expect("timestamp")
}

fn day(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("date")
}

#[test]
fn a_week_is_seven_local_days_ending_today() {
    let w = stats_window(StatsPeriod::Week, at("2026-10-07 22:30"), 180, None);
    assert_eq!(w.to_local, day("2026-10-08"));
    assert_eq!(w.from_local, day("2026-10-02"));
    assert_eq!(w.from, at("2026-10-01 21:00"));
    assert_eq!(w.to, at("2026-10-08 21:00"));
    assert_eq!(w.previous_from, Some(at("2026-09-24 21:00")));
    assert_eq!(w.unit, StatsUnit::Day);
}

#[test]
fn a_month_is_thirty_days_and_a_year_twelve_calendar_months() {
    let now = at("2026-10-07 12:00");
    let month = stats_window(StatsPeriod::Month, now, 0, None);
    assert_eq!(month.from_local, day("2026-09-08"));
    assert_eq!(month.previous_from, Some(at("2026-08-09 00:00")));
    let year = stats_window(StatsPeriod::Year, now, -300, None);
    assert_eq!(year.from_local, day("2025-11-01"));
    assert_eq!(year.from, at("2025-11-01 05:00"));
    assert_eq!(year.previous_from, Some(at("2024-11-01 05:00")));
    assert_eq!(year.unit, StatsUnit::Month);
}

#[test]
fn all_time_starts_at_the_month_of_the_first_play() {
    let now = at("2026-10-07 12:00");
    let w = stats_window(StatsPeriod::All, now, 0, Some(at("2025-03-17 08:00")));
    assert_eq!(w.from_local, day("2025-03-01"));
    assert_eq!(w.previous_from, None);
    let empty = stats_window(StatsPeriod::All, now, 0, None);
    assert_eq!(empty.from_local, day("2026-10-01"));
}

#[test]
fn offsets_are_clamped_to_real_time_zones() {
    assert_eq!(clamp_utc_offset(100_000), 840);
    assert_eq!(clamp_utc_offset(-100_000), -840);
    assert_eq!(clamp_utc_offset(330), 330);
}

async fn play(
    pool: &PgPool,
    track: &str,
    artist: (&str, Option<&str>),
    duration: i32,
    played_at: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO listening_history (soundcloud_user_id, sc_track_id, title, artist_name, artist_urn, duration, played_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(USER)
    .bind(format!("soundcloud:tracks:{track}"))
    .bind(format!("Song {track}"))
    .bind(artist.0)
    .bind(artist.1)
    .bind(duration)
    .bind(at(played_at))
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn stats_rank_tops_and_bucket_by_local_time(pool: PgPool) -> anyhow::Result<()> {
    let a = ("Alpha", Some("soundcloud:users:1"));
    let b = ("Beta", None);
    play(&pool, "1", a, 200_000, "2026-10-06 23:30").await?;
    play(&pool, "1", a, 200_000, "2026-10-07 10:00").await?;
    play(&pool, "2", a, 100_000, "2026-10-07 11:00").await?;
    play(&pool, "3", b, 300_000, "2026-10-05 09:00").await?;
    play(&pool, "3", b, 300_000, "2026-09-25 09:00").await?;
    play(&pool, "4", ("", None), 50_000, "2026-10-07 11:30").await?;

    let service = HistoryService::new(pool);
    let stats = service
        .stats(USER, StatsPeriod::Week, 60, at("2026-10-07 12:00"))
        .await?;

    assert_eq!(stats.totals.plays, 5);
    assert_eq!(stats.totals.listened_ms, 850_000);
    assert_eq!(stats.totals.tracks, 4);
    assert_eq!(stats.previous.as_ref().map(|p| p.plays), Some(1));

    let tracks: Vec<(&str, i64)> = stats
        .top_tracks
        .iter()
        .map(|t| (t.track_urn.as_str(), t.plays))
        .collect();
    assert_eq!(tracks[0], ("soundcloud:tracks:1", 2));
    assert_eq!(stats.top_artists.len(), 2);
    assert_eq!(stats.top_artists[0].artist_name, "Alpha");
    assert_eq!(stats.top_artists[0].plays, 3);
    assert_eq!(stats.top_artists[0].tracks, 2);
    assert_eq!(stats.top_artists[1].artist_urn, None);

    let days: Vec<(NaiveDate, i64)> = stats.timeline.iter().map(|p| (p.date, p.plays)).collect();
    assert_eq!(days, vec![(day("2026-10-05"), 1), (day("2026-10-07"), 4)]);

    let late = stats
        .rhythm
        .iter()
        .find(|c| c.hour == 0)
        .expect("the 23:30 UTC play lands at 00:30 local");
    assert_eq!((late.weekday, late.plays), (3, 1));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_empty_history_returns_zeroes(pool: PgPool) -> anyhow::Result<()> {
    let service = HistoryService::new(pool);
    let stats = service
        .stats(USER, StatsPeriod::All, 0, at("2026-10-07 12:00"))
        .await?;
    assert_eq!(stats.totals.plays, 0);
    assert!(stats.previous.is_none());
    assert!(stats.top_tracks.is_empty() && stats.timeline.is_empty());
    Ok(())
}
