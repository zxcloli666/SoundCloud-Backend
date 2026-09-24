use std::future::Future;
use std::time::Duration;

use crate::error::{AppError, AppResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchStrategy {
    Fallback,
    Race,
    Hedge,
}

impl FetchStrategy {
    pub fn from_env() -> Self {
        match std::env::var("CALL_FETCH_STRATEGY").as_deref() {
            Ok("fallback") => Self::Fallback,
            Ok("race") => Self::Race,
            _ => Self::Hedge,
        }
    }
}

pub async fn within_budget<T>(
    budget: Duration,
    call: impl Future<Output = AppResult<T>>,
) -> AppResult<T> {
    match tokio::time::timeout(budget, call).await {
        Ok(result) => result,
        Err(_) => Err(AppError::sc_deadline_exceeded()),
    }
}

pub async fn hedge<T, E, P, B>(primary: P, delay: Duration, backup: B) -> AppResult<T>
where
    E: Into<AppError>,
    P: Future<Output = Result<T, E>>,
    B: Future<Output = Result<T, E>>,
{
    tokio::pin!(primary);
    match tokio::time::timeout(delay, &mut primary).await {
        Ok(Ok(v)) => return Ok(v),
        Ok(Err(_)) => return backup.await.map_err(Into::into),
        Err(_) => {}
    }
    first_success(primary, backup).await
}

pub async fn race<T, E, P, B>(primary: P, backup: B) -> AppResult<T>
where
    E: Into<AppError>,
    P: Future<Output = Result<T, E>>,
    B: Future<Output = Result<T, E>>,
{
    tokio::pin!(primary);
    first_success(primary, backup).await
}

async fn first_success<T, E, P, B>(mut primary: std::pin::Pin<&mut P>, backup: B) -> AppResult<T>
where
    E: Into<AppError>,
    P: Future<Output = Result<T, E>>,
    B: Future<Output = Result<T, E>>,
{
    tokio::pin!(backup);
    let mut perr: Option<AppError> = None;
    let mut berr: Option<AppError> = None;
    loop {
        tokio::select! {
            r = &mut primary, if perr.is_none() => match r {
                Ok(v) => return Ok(v),
                Err(e) => perr = Some(e.into()),
            },
            r = &mut backup, if berr.is_none() => match r {
                Ok(v) => return Ok(v),
                Err(e) => berr = Some(e.into()),
            },
        }
        if perr.is_some() && berr.is_some() {
            return Err(berr
                .take()
                .or_else(|| perr.take())
                .unwrap_or_else(|| AppError::internal("orchestration: both channels failed")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const BUDGET: Duration = Duration::from_secs(20);

    async fn slow(answer: &'static str, takes: Duration) -> AppResult<Value> {
        tokio::time::sleep(takes).await;
        Ok(Value::from(answer))
    }

    #[tokio::test(start_paused = true)]
    async fn hedge_returns_primary_when_fast() {
        let r = hedge(
            async { Ok::<_, AppError>(Value::from("p")) },
            Duration::from_millis(50),
            async { Ok(Value::from("b")) },
        )
        .await;
        assert_eq!(r.unwrap(), Value::from("p"));
    }

    #[tokio::test(start_paused = true)]
    async fn hedge_falls_back_when_primary_fails_fast() {
        let r = hedge(
            async { Err::<Value, _>(AppError::internal("x")) },
            Duration::from_millis(50),
            async { Ok(Value::from("b")) },
        )
        .await;
        assert_eq!(r.unwrap(), Value::from("b"));
    }

    #[tokio::test(start_paused = true)]
    async fn hedge_backup_wins_when_primary_slow() {
        let started = tokio::time::Instant::now();
        let r = hedge(
            slow("p", Duration::from_millis(500)),
            Duration::from_millis(20),
            async { Ok(Value::from("b")) },
        )
        .await;
        assert_eq!(r.unwrap(), Value::from("b"));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test(start_paused = true)]
    async fn race_first_success_wins() {
        let r = race(slow("p", Duration::from_millis(100)), async {
            Ok(Value::from("b"))
        })
        .await;
        assert_eq!(r.unwrap(), Value::from("b"));
    }

    #[tokio::test(start_paused = true)]
    async fn race_both_fail_returns_err() {
        let r: AppResult<Value> = race(async { Err(AppError::internal("p")) }, async {
            Err(AppError::internal("b"))
        })
        .await;
        assert!(r.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_relay_and_a_slow_backup_share_one_budget_instead_of_each_paying_its_own() {
        let started = tokio::time::Instant::now();
        let result = within_budget(
            BUDGET,
            hedge(
                slow("relay", Duration::from_secs(120)),
                Duration::from_millis(700),
                slow("backup", Duration::from_secs(120)),
            ),
        )
        .await;

        assert!(matches!(
            result.expect_err("a call that nobody answers must end"),
            AppError::ScDeadlineExceeded
        ));
        assert_eq!(started.elapsed(), BUDGET);
    }

    #[tokio::test(start_paused = true)]
    async fn a_chain_of_slow_tokens_cannot_outlive_the_budget_by_retrying() {
        let started = tokio::time::Instant::now();
        let result = within_budget(BUDGET, async {
            for _ in 0..8 {
                slow("token", Duration::from_secs(30)).await?;
            }
            Ok(Value::from("never"))
        })
        .await;

        assert!(matches!(
            result.expect_err("retries must share the budget"),
            AppError::ScDeadlineExceeded
        ));
        assert_eq!(started.elapsed(), BUDGET);
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_that_answers_inside_the_budget_is_not_cut_short() {
        let answer = within_budget(BUDGET, slow("relay", Duration::from_secs(19)))
            .await
            .expect("answers in time");
        assert_eq!(answer, Value::from("relay"));
    }

    #[tokio::test(start_paused = true)]
    async fn an_exhausted_budget_is_a_gateway_timeout_with_its_own_code() {
        let error = within_budget(
            Duration::from_secs(1),
            slow("relay", Duration::from_secs(5)),
        )
        .await
        .expect_err("times out");
        assert_eq!(error.status(), axum::http::StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(error.public_code(), "soundcloud_read_timed_out");
        let response = axum::response::IntoResponse::into_response(error);
        assert_eq!(
            response.headers().get(axum::http::header::RETRY_AFTER),
            Some(&axum::http::HeaderValue::from_static("15")),
            "a timeout must tell the client when to come back"
        );
    }
}
