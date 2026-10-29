use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use backend_contracts::diag::{Bundle, EventsBody};
use backend_contracts::diag_report::{IssueStats, Report, View};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

use super::code;
use super::jaeger::{self, TraceView};
use super::report::{self, Sourcemaps};
use crate::error::AppError;

const RETENTION_DAYS: i64 = 30;
const CODE_ATTEMPTS: usize = 8;
const TITLE_CHARS: usize = 300;
const STATS_HOURS: i32 = 720;

pub struct DiagService {
    pg: PgPool,
    jaeger_url: String,
    http: wreq::Client,
    sourcemaps: Sourcemaps,
}

#[derive(Debug, Serialize)]
pub struct Issue {
    pub fingerprint: String,
    pub kind: String,
    pub title: String,
    pub events: i64,
    pub installs: i64,
    pub first_at: DateTime<Utc>,
    pub last_at: DateTime<Utc>,
    pub first_version: String,
    pub versions: BTreeMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub struct Day {
    pub day: DateTime<Utc>,
    pub events: i64,
    pub installs: i64,
}

#[derive(Debug, Serialize)]
pub struct IssueDetail {
    #[serde(flatten)]
    pub issue: Issue,
    pub days: Vec<Day>,
}

impl DiagService {
    pub fn new(pg: PgPool, jaeger_url: String, sourcemaps_dir: PathBuf) -> Arc<Self> {
        let http = wreq::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("a plain http client builds");
        Arc::new(Self {
            pg,
            jaeger_url,
            http,
            sourcemaps: Sourcemaps::new(sourcemaps_dir),
        })
    }

    pub async fn store_bundle(&self, bundle: &Bundle) -> Result<(String, DateTime<Utc>), AppError> {
        let body = serde_json::to_value(bundle).map_err(|e| AppError::internal(e.to_string()))?;
        for _ in 0..CODE_ATTEMPTS {
            let code = code::generate();
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
                return Ok((code, row.at + ChronoDuration::days(RETENTION_DAYS)));
            }
        }
        Err(AppError::internal("no free diagnostics code"))
    }

    pub async fn store_events(&self, body: &EventsBody) -> Result<(), AppError> {
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
            .execute(&self.pg)
            .await?;
        }
        Ok(())
    }

    pub async fn bundle(&self, code: &str) -> Result<Option<Value>, AppError> {
        let Some(code) = code::normalize(code) else {
            return Ok(None);
        };
        let row = sqlx::query_file!("queries/diag/bundle.sql", code)
            .fetch_optional(&self.pg)
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
        let Some(overall) = self.collect(STATS_HOURS, 1, Some(fingerprint)).await?.pop() else {
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
            first_at: overall.first_at.to_rfc3339(),
            last_at: overall.last_at.to_rfc3339(),
            first_version: overall.first_version,
            versions: overall.versions,
        }))
    }

    pub async fn issues(&self, hours: i32, limit: i64) -> Result<Vec<Issue>, AppError> {
        self.collect(hours, limit, None).await
    }

    pub async fn issue(
        &self,
        fingerprint: &str,
        hours: i32,
    ) -> Result<Option<IssueDetail>, AppError> {
        let Some(issue) = self.collect(hours, 1, Some(fingerprint)).await?.pop() else {
            return Ok(None);
        };
        let days = sqlx::query_file!("queries/diag/issue_days.sql", hours, fingerprint)
            .fetch_all(&self.pg)
            .await?
            .into_iter()
            .map(|row| Day {
                day: row.day,
                events: row.events,
                installs: row.installs,
            })
            .collect();
        Ok(Some(IssueDetail { issue, days }))
    }

    async fn collect(
        &self,
        hours: i32,
        limit: i64,
        only: Option<&str>,
    ) -> Result<Vec<Issue>, AppError> {
        let rows = sqlx::query_file!("queries/diag/issues.sql", hours, limit, only)
            .fetch_all(&self.pg)
            .await?;
        let fingerprints: Vec<String> = rows.iter().map(|row| row.fingerprint.clone()).collect();
        let counts = sqlx::query_file!("queries/diag/issue_versions.sql", hours, &fingerprints)
            .fetch_all(&self.pg)
            .await?;
        let mut versions: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
        for row in counts {
            versions
                .entry(row.fingerprint)
                .or_default()
                .insert(row.app_version, row.events);
        }
        Ok(rows
            .into_iter()
            .map(|row| Issue {
                versions: versions.remove(&row.fingerprint).unwrap_or_default(),
                fingerprint: row.fingerprint,
                kind: row.kind,
                title: row.title,
                events: row.events,
                installs: row.installs,
                first_at: row.first_at,
                last_at: row.last_at,
                first_version: row.first_version,
            })
            .collect())
    }
}
