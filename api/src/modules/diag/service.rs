use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use backend_contracts::diag::{Bundle, EventsBody};
use backend_contracts::diag_report::{
    Issue, IssueDay, IssueDetail, IssueEvent, IssueStats, Report, View,
};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, SecondsFormat, Utc};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use sqlx::{PgPool, Postgres, Transaction};

use super::code;
use super::jaeger::{self, TraceView};
use super::report::{self, Sourcemaps};
use crate::common::admission::ClientIdentity;
use crate::config::DiagCfg;
use crate::error::AppError;

const BUNDLE_RETENTION_DAYS: i64 = 30;
const CODE_ATTEMPTS: usize = 8;
const TITLE_CHARS: usize = 300;
const STATS_DAYS: i64 = 30;
const SIZE_CHECK_EVERY: Duration = Duration::from_secs(60);

pub struct DiagService {
    pg: PgPool,
    jaeger_url: String,
    http: wreq::Client,
    sourcemaps: Sourcemaps,
    host: String,
    code_slot: u8,
    max_bytes: u64,
    origin_key: Vec<u8>,
    size: Mutex<Option<(Instant, u64)>>,
}

impl DiagService {
    pub fn new(pg: PgPool, config: &DiagCfg) -> Arc<Self> {
        let http = wreq::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("a plain http client builds");
        Arc::new(Self {
            pg,
            jaeger_url: config.jaeger_url.clone(),
            http,
            sourcemaps: Sourcemaps::new(config.sourcemaps_dir.clone().into()),
            host: config.host.clone(),
            code_slot: config.code_slot,
            max_bytes: config.max_bytes,
            origin_key: config.origin_key.expose().as_bytes().to_vec(),
            size: Mutex::new(None),
        })
    }

    pub async fn ensure_room(&self) -> Result<(), AppError> {
        let cached = *self.size.lock().expect("the size lock is never poisoned");
        let bytes = match cached {
            Some((checked, bytes)) if checked.elapsed() < SIZE_CHECK_EVERY => bytes,
            _ => {
                let row = sqlx::query_file!("queries/diag/size.sql")
                    .fetch_one(&self.pg)
                    .await?;
                let bytes = u64::try_from(row.bytes).unwrap_or(0);
                *self.size.lock().expect("the size lock is never poisoned") =
                    Some((Instant::now(), bytes));
                bytes
            }
        };
        if bytes > self.max_bytes {
            return Err(AppError::service_unavailable("diagnostics storage is full"));
        }
        Ok(())
    }

    pub async fn store_bundle(&self, bundle: &Bundle) -> Result<(String, DateTime<Utc>), AppError> {
        let body = serde_json::to_value(bundle).map_err(|e| AppError::internal(e.to_string()))?;
        for _ in 0..CODE_ATTEMPTS {
            let code = code::generate(self.code_slot);
            let stored = sqlx::query_file!(
                "queries/diag/insert_bundle.sql",
                code,
                bundle.app.version,
                bundle.app.os,
                body
            )
            .fetch_optional(&self.pg)
            .await?;
            if let Some(row) = stored {
                return Ok((code, row.at + ChronoDuration::days(BUNDLE_RETENTION_DAYS)));
            }
        }
        Err(AppError::internal("no free diagnostics code"))
    }

    pub async fn store_events(
        &self,
        body: &EventsBody,
        client: &ClientIdentity,
    ) -> Result<(), AppError> {
        let origin = self.origin(client);
        let mut transaction = self.pg.begin().await?;
        for event in &body.events {
            let title: String = event
                .message
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(TITLE_CHARS)
                .collect();
            let json =
                serde_json::to_value(event).map_err(|e| AppError::internal(e.to_string()))?;
            sqlx::query_file!(
                "queries/diag/insert_event.sql",
                event.fingerprint,
                event.kind.as_str(),
                title,
                body.app.version,
                body.app.os,
                body.app.id,
                event.trace,
                json
            )
            .execute(&mut *transaction)
            .await?;
            sqlx::query_file!(
                "queries/diag/rollup_day.sql",
                event.fingerprint,
                self.host,
                body.app.version,
                event.kind.as_str(),
                title
            )
            .execute(&mut *transaction)
            .await?;
            sqlx::query_file!(
                "queries/diag/rollup_origin.sql",
                event.fingerprint,
                self.host,
                origin
            )
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn bundle(&self, code: &str) -> Result<Option<Value>, AppError> {
        let Some(code) = code::normalize(code) else {
            return Ok(None);
        };
        let mut transaction = self.timed().await?;
        let row = sqlx::query_file!("queries/diag/bundle.sql", code)
            .fetch_optional(&mut *transaction)
            .await?;
        Ok(row.map(|row| row.body))
    }

    pub async fn trace(&self, trace: &str) -> Result<Option<TraceView>, AppError> {
        jaeger::fetch(&self.http, &self.jaeger_url, trace).await
    }

    pub fn sourcemaps(&self) -> &Sourcemaps {
        &self.sourcemaps
    }

    pub async fn report(&self, code: &str, view: View) -> Result<Option<Report>, AppError> {
        report::build(self, code, view).await
    }

    pub async fn issue_stats(&self, fingerprint: &str) -> Result<Option<IssueStats>, AppError> {
        let overall = self
            .collect(STATS_DAYS * 24, 1, Some(fingerprint))
            .await?
            .pop();
        let Some(overall) = overall else {
            return Ok(None);
        };
        let installs_24h = self
            .collect(24, 1, Some(fingerprint))
            .await?
            .pop()
            .map_or(0, |recent| recent.installs);
        Ok(Some(IssueStats {
            events: overall.events,
            installs: overall.installs,
            installs_24h,
            first_at: overall.first_at,
            last_at: overall.last_at,
            first_version: overall.first_version,
            versions: overall.versions,
        }))
    }

    pub async fn issues(&self, hours: i32, limit: i64) -> Result<Vec<Issue>, AppError> {
        self.collect(i64::from(hours), limit, None).await
    }

    pub async fn issue(
        &self,
        fingerprint: &str,
        hours: i32,
    ) -> Result<Option<IssueDetail>, AppError> {
        let Some(issue) = self
            .collect(i64::from(hours), 1, Some(fingerprint))
            .await?
            .pop()
        else {
            return Ok(None);
        };
        let since = window_start(i64::from(hours));
        let mut transaction = self.timed().await?;
        let installs: BTreeMap<NaiveDate, i64> =
            sqlx::query_file!("queries/diag/issue_origins_daily.sql", since, fingerprint)
                .fetch_all(&mut *transaction)
                .await?
                .into_iter()
                .map(|row| (row.day, row.installs))
                .collect();
        let events = sqlx::query_file!("queries/diag/issue_events.sql", fingerprint)
            .fetch_all(&mut *transaction)
            .await?;
        drop(transaction);
        let days = window_days(since)
            .into_iter()
            .zip(&issue.daily)
            .map(|(day, events)| IssueDay {
                installs: installs.get(&day).copied().unwrap_or(0),
                day: day.to_string(),
                events: *events,
            })
            .collect();
        let events = events
            .into_iter()
            .map(|row| IssueEvent {
                at: row.at.to_rfc3339_opts(SecondsFormat::Millis, true),
                app_version: row.app_version,
                os: row.os,
                trace: row.trace,
                context: row
                    .context
                    .and_then(|context| serde_json::from_value(context).ok())
                    .unwrap_or_default(),
            })
            .collect();
        Ok(Some(IssueDetail {
            issue,
            days,
            events,
        }))
    }

    async fn collect(
        &self,
        hours: i64,
        limit: i64,
        only: Option<&str>,
    ) -> Result<Vec<Issue>, AppError> {
        let since = window_start(hours);
        let mut transaction = self.timed().await?;
        let rows = sqlx::query_file!("queries/diag/issues.sql", since, limit, only)
            .fetch_all(&mut *transaction)
            .await?;
        let fingerprints: Vec<String> = rows.iter().map(|row| row.fingerprint.clone()).collect();
        let counts = sqlx::query_file!("queries/diag/issue_versions.sql", since, &fingerprints)
            .fetch_all(&mut *transaction)
            .await?;
        let daily = sqlx::query_file!("queries/diag/issue_daily.sql", since, &fingerprints)
            .fetch_all(&mut *transaction)
            .await?;
        drop(transaction);
        let mut versions: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
        for row in counts {
            versions
                .entry(row.fingerprint)
                .or_default()
                .insert(row.app_version, row.events);
        }
        let days = window_days(since);
        let mut per_day: BTreeMap<String, BTreeMap<NaiveDate, i64>> = BTreeMap::new();
        for row in daily {
            per_day
                .entry(row.fingerprint)
                .or_default()
                .insert(row.day, row.events);
        }
        Ok(rows
            .into_iter()
            .map(|row| {
                let counted = per_day.remove(&row.fingerprint).unwrap_or_default();
                Issue {
                    versions: versions.remove(&row.fingerprint).unwrap_or_default(),
                    daily: days
                        .iter()
                        .map(|day| counted.get(day).copied().unwrap_or(0))
                        .collect(),
                    fingerprint: row.fingerprint,
                    kind: row.kind,
                    title: row.title,
                    events: row.events,
                    installs: row.installs,
                    first_at: row.first_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                    last_at: row.last_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                    first_version: row.first_version,
                }
            })
            .collect())
    }

    pub(super) async fn timed(&self) -> Result<Transaction<'static, Postgres>, AppError> {
        let mut transaction = self.pg.begin().await?;
        sqlx::query("SET LOCAL statement_timeout = '2s'")
            .execute(&mut *transaction)
            .await?;
        Ok(transaction)
    }

    fn origin(&self, client: &ClientIdentity) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.origin_key)
            .expect("an hmac accepts a key of any length");
        mac.update(client.0.as_bytes());
        hex::encode(&mac.finalize().into_bytes()[..16])
    }
}

fn window_start(hours: i64) -> NaiveDate {
    (Utc::now() - ChronoDuration::hours(hours)).date_naive()
}

fn window_days(since: NaiveDate) -> Vec<NaiveDate> {
    let today = Utc::now().date_naive();
    since.iter_days().take_while(|day| *day <= today).collect()
}
