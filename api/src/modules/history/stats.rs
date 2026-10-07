use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::common::sc_ids::{EntityRef, user_id_variants};
use crate::error::AppResult;
use crate::modules::history::service::HistoryService;

const TOP_LIMIT: i64 = 10;
const MAX_UTC_OFFSET_MIN: i32 = 14 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StatsPeriod {
    Week,
    #[default]
    Month,
    Year,
    All,
}

impl StatsPeriod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatsUnit {
    Day,
    Month,
}

impl StatsUnit {
    fn as_sql(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Month => "month",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatsWindow {
    pub from_local: NaiveDate,
    pub to_local: NaiveDate,
    pub from: NaiveDateTime,
    pub to: NaiveDateTime,
    pub previous_from: Option<NaiveDateTime>,
    pub unit: StatsUnit,
}

pub fn clamp_utc_offset(minutes: i32) -> i32 {
    minutes.clamp(-MAX_UTC_OFFSET_MIN, MAX_UTC_OFFSET_MIN)
}

pub fn stats_window(
    period: StatsPeriod,
    now: NaiveDateTime,
    utc_offset_min: i32,
    first_played_at: Option<NaiveDateTime>,
) -> StatsWindow {
    let offset = Duration::minutes(i64::from(utc_offset_min));
    let today = (now + offset).date();
    let month_start = |d: NaiveDate| d.with_day(1).unwrap_or(d);
    let (from_local, previous_local, unit) = match period {
        StatsPeriod::Week => (
            today - Duration::days(6),
            Some(today - Duration::days(13)),
            StatsUnit::Day,
        ),
        StatsPeriod::Month => (
            today - Duration::days(29),
            Some(today - Duration::days(59)),
            StatsUnit::Day,
        ),
        StatsPeriod::Year => {
            let start = month_start(today)
                .checked_sub_months(Months::new(11))
                .unwrap_or(today);
            (
                start,
                start.checked_sub_months(Months::new(12)),
                StatsUnit::Month,
            )
        }
        StatsPeriod::All => {
            let first = first_played_at.map_or(today, |at| (at + offset).date());
            (month_start(first), None, StatsUnit::Month)
        }
    };
    let to_utc = |d: NaiveDate| d.and_time(NaiveTime::MIN) - offset;
    StatsWindow {
        from_local,
        to_local: today,
        from: to_utc(from_local),
        to: to_utc(today + Duration::days(1)),
        previous_from: previous_local.map(to_utc),
        unit,
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsTotals {
    pub plays: i64,
    pub listened_ms: i64,
    pub tracks: i64,
    pub artists: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopTrack {
    pub track_urn: String,
    pub title: String,
    pub artist_name: String,
    pub artist_urn: Option<String>,
    pub artwork_url: Option<String>,
    pub duration: i32,
    pub plays: i64,
    pub listened_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopArtist {
    pub artist_name: String,
    pub artist_urn: Option<String>,
    pub artwork_url: Option<String>,
    pub plays: i64,
    pub tracks: i64,
    pub listened_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelinePoint {
    pub date: NaiveDate,
    pub plays: i64,
    pub listened_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RhythmCell {
    pub weekday: i32,
    pub hour: i32,
    pub plays: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListeningStats {
    pub period: StatsPeriod,
    pub unit: StatsUnit,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub totals: StatsTotals,
    pub previous: Option<StatsTotals>,
    pub top_tracks: Vec<TopTrack>,
    pub top_artists: Vec<TopArtist>,
    pub timeline: Vec<TimelinePoint>,
    pub rhythm: Vec<RhythmCell>,
}

impl HistoryService {
    pub async fn stats(
        &self,
        sc_user_id: &str,
        period: StatsPeriod,
        utc_offset_min: i32,
        now: NaiveDateTime,
    ) -> AppResult<ListeningStats> {
        let variants = user_id_variants(sc_user_id);
        let first_played_at = match period {
            StatsPeriod::All => {
                sqlx::query_file_scalar!("queries/history/stats/first_play.sql", &variants)
                    .fetch_one(&self.pg)
                    .await?
            }
            _ => None,
        };
        let window = stats_window(period, now, utc_offset_min, first_played_at);
        let totals = self.totals(&variants, window.from, window.to).await?;
        let previous = match window.previous_from {
            Some(from) => Some(self.totals(&variants, from, window.from).await?),
            None => None,
        };
        if totals.plays == 0 {
            return Ok(ListeningStats {
                period,
                unit: window.unit,
                from: window.from_local,
                to: window.to_local,
                totals,
                previous,
                top_tracks: Vec::new(),
                top_artists: Vec::new(),
                timeline: Vec::new(),
                rhythm: Vec::new(),
            });
        }
        let top_tracks = sqlx::query_file!(
            "queries/history/stats/top_tracks.sql",
            &variants,
            window.from,
            window.to,
            TOP_LIMIT
        )
        .fetch_all(&self.pg)
        .await?
        .into_iter()
        .map(|row| TopTrack {
            track_urn: EntityRef::track(&row.sc_track_id).map_or(row.sc_track_id, EntityRef::urn),
            title: row.title,
            artist_name: row.artist_name,
            artist_urn: row.artist_urn,
            artwork_url: row.artwork_url,
            duration: row.duration,
            plays: row.plays,
            listened_ms: row.listened_ms,
        })
        .collect();
        let top_artists = sqlx::query_file_as!(
            TopArtist,
            "queries/history/stats/top_artists.sql",
            &variants,
            window.from,
            window.to,
            TOP_LIMIT
        )
        .fetch_all(&self.pg)
        .await?;
        let timeline = sqlx::query_file!(
            "queries/history/stats/timeline.sql",
            &variants,
            window.from,
            window.to,
            window.unit.as_sql(),
            utc_offset_min
        )
        .fetch_all(&self.pg)
        .await?
        .into_iter()
        .map(|row| TimelinePoint {
            date: row.bucket.date(),
            plays: row.plays,
            listened_ms: row.listened_ms,
        })
        .collect();
        let rhythm = sqlx::query_file_as!(
            RhythmCell,
            "queries/history/stats/rhythm.sql",
            &variants,
            window.from,
            window.to,
            utc_offset_min
        )
        .fetch_all(&self.pg)
        .await?;
        Ok(ListeningStats {
            period,
            unit: window.unit,
            from: window.from_local,
            to: window.to_local,
            totals,
            previous,
            top_tracks,
            top_artists,
            timeline,
            rhythm,
        })
    }

    async fn totals(
        &self,
        variants: &[String],
        from: NaiveDateTime,
        to: NaiveDateTime,
    ) -> AppResult<StatsTotals> {
        Ok(sqlx::query_file_as!(
            StatsTotals,
            "queries/history/stats/totals.sql",
            variants,
            from,
            to
        )
        .fetch_one(&self.pg)
        .await?)
    }
}

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;
