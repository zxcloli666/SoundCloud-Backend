use std::time::Duration;

use super::*;
use crate::error::AppError;

#[test]
fn a_rendered_snapshot_carries_route_labels_and_no_identifiers() {
    use crate::common::http_metrics::route_label;

    init();
    record_request(
        "GET",
        "/tracks/{urn}".to_owned(),
        200,
        Duration::from_millis(12),
    );
    record_request("GET", route_label(None), 404, Duration::from_millis(3));
    record_dependency(
        "soundcloud",
        "track_by_id",
        Outcome::Error,
        Duration::from_millis(40),
    );

    let Some(handle) = HANDLE.get() else {
        return;
    };
    let rendered = handle.render();

    assert!(rendered.contains(REQUEST_DURATION));
    assert!(rendered.contains("route=\"/tracks/{urn}\""));
    assert!(rendered.contains("route=\"unmatched\""));
    assert!(rendered.contains("status=\"404\""));
    assert!(rendered.contains("dependency=\"soundcloud\""));
    assert!(rendered.contains("outcome=\"error\""));
}

#[test]
fn a_storm_of_unrouted_paths_mints_one_series_not_a_thousand() {
    use crate::common::http_metrics::route_label;

    init();
    for nonce in 0..256 {
        let _ = format!("/no-such-route-{nonce}");
        record_request("GET", route_label(None), 404, Duration::from_millis(1));
    }

    let Some(handle) = HANDLE.get() else {
        return;
    };
    let rendered = handle.render();

    assert!(
        !rendered.contains("no-such-route"),
        "a requested path must never reach a label"
    );
    let series = rendered
        .lines()
        .filter(|line| line.starts_with(REQUESTS_TOTAL) && line.contains("route=\"unmatched\""))
        .count();
    assert_eq!(
        series, 1,
        "every unrouted request must land on the same series"
    );
}

#[test]
#[allow(deprecated)]
fn a_repeated_series_finds_its_registry_entry_instead_of_adding_one() {
    use std::hash::{BuildHasher, BuildHasherDefault};

    let key = metrics::Key::from_parts(
        REQUEST_DURATION,
        vec![metrics::Label::new("route", "/tracks/{urn}")],
    );
    let stored = BuildHasherDefault::<metrics::KeyHasher>::default().hash_one(&key);

    assert_eq!(
        stored,
        key.get_hash(),
        "the registry stores a series under one hash and looks it up under another, \
         so every call adds an entry that is never freed"
    );
}

#[test]
fn a_soundcloud_failure_is_counted_by_class_and_its_retry_after_is_kept() {
    init();
    record_sc_failure(&AppError::ScApi {
        status: 429,
        body: serde_json::Value::Null,
        retry_after_sec: Some(75),
    });
    record_sc_failure(&AppError::ScApi {
        status: 503,
        body: serde_json::Value::Null,
        retry_after_sec: None,
    });
    record_sc_failure(&AppError::ScDeadlineExceeded);

    let Some(handle) = HANDLE.get() else {
        return;
    };
    let rendered = handle.render();

    assert!(rendered.contains("class=\"rate_limited\""));
    assert!(rendered.contains("class=\"upstream_5xx\""));
    assert!(rendered.contains("class=\"unreachable\""));
    assert!(
        rendered.contains("api_sc_retry_after_seconds_bucket"),
        "the Retry-After SoundCloud asked for must be measurable, not only obeyed"
    );
    assert!(
        rendered.contains("api_sc_retry_after_seconds_count 1"),
        "only the rate-limited failure carried a Retry-After, so exactly one sample is expected"
    );
}

#[test]
fn every_outcome_has_a_stable_name() {
    assert_eq!(
        [
            Outcome::Ok.as_str(),
            Outcome::Miss.as_str(),
            Outcome::Error.as_str(),
            Outcome::Timeout.as_str(),
        ],
        ["ok", "miss", "error", "timeout"]
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_scrape_reports_the_database_it_actually_serves(
    pool: sqlx::PgPool,
) -> anyhow::Result<()> {
    init();

    let Some(body) = render(&pool).await else {
        return Ok(());
    };

    for metric in [
        "api_pg_backends",
        "api_pg_transactions_total",
        "api_pg_deadlocks_total",
        "api_pg_blocks_total",
        "api_pg_sessions",
        "api_pg_pool_connections",
        "api_pg_pool_wait_seconds",
        "api_pg_pool_wait_last_seconds",
    ] {
        assert!(body.contains(metric), "{metric} is missing from a scrape");
    }
    assert!(body.contains("outcome=\"rollback\""));
    assert!(body.contains("source=\"disk\""));
    assert!(
        body.contains("api_pg_pool_wait_seconds_bucket{outcome=\"ok\""),
        "a healthy pool must report a measured wait, not an empty histogram"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_pool_that_cannot_hand_out_a_connection_is_visible_as_such(
    pool: sqlx::PgPool,
) -> anyhow::Result<()> {
    init();
    pool.close().await;

    sample_pool_wait(&pool).await;
    let Some(body) = render(&pool).await else {
        return Ok(());
    };

    assert!(
        body.contains("api_pg_pool_wait_seconds_bucket{outcome=\"error\"")
            || body.contains("api_pg_pool_wait_seconds_bucket{outcome=\"timeout\""),
        "a pool that refuses a connection must not read as a healthy wait"
    );
    Ok(())
}

fn ops_artifacts() -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ops");
    let mut joined = String::new();
    let entries = std::fs::read_dir(&dir).expect("the ops artifacts ship with the repository");
    for entry in entries.flatten() {
        if let Ok(body) = std::fs::read_to_string(entry.path()) {
            joined.push_str(&body);
            joined.push('\n');
        }
    }
    assert!(
        joined.contains("api_http_requests_total"),
        "the ops artifacts were not read, so this test proves nothing"
    );
    joined
}

fn exported(body: &str, name: &str) -> bool {
    body.lines().any(|line| {
        line.strip_prefix(name).is_some_and(|rest| {
            rest.starts_with(' ')
                || rest.starts_with('{')
                || rest.starts_with("_bucket")
                || rest.starts_with("_sum")
                || rest.starts_with("_count")
        })
    })
}

fn metrics_named_in(rules: &str, prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for token in rules.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if !token.starts_with(prefix) || token.len() <= prefix.len() + 2 {
            continue;
        }
        let base = ["_bucket", "_count", "_sum"]
            .iter()
            .find_map(|suffix| token.strip_suffix(suffix))
            .unwrap_or(token);
        if !names.iter().any(|seen| seen == base) {
            names.push(base.to_owned());
        }
    }
    names
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn every_metric_an_alert_watches_is_actually_exported(
    pool: sqlx::PgPool,
) -> anyhow::Result<()> {
    use crate::common::http_metrics::route_label;

    init();
    record_request("GET", route_label(None), 200, Duration::from_millis(1));
    record_dependency("redis", "get", Outcome::Ok, Duration::from_millis(1));
    record_sc_tier("relay_lua", "track", Outcome::Ok, Duration::from_millis(1));
    record_sc_failure(&AppError::ScApi {
        status: 429,
        body: serde_json::Value::Null,
        retry_after_sec: Some(30),
    });
    set_relay_breaker_open("relay_lua", false);
    record_session_miss("in_flight");
    sample_pool_wait(&pool).await;

    let Some(body) = render(&pool).await else {
        return Ok(());
    };

    let watched = metrics_named_in(&ops_artifacts(), "api_");
    assert!(
        watched.len() >= 10,
        "the ops artifacts must actually reference api metrics, found {watched:?}"
    );
    for name in watched {
        assert!(
            exported(&body, &name),
            "an alert rule or dashboard panel watches {name}, which no scrape exports"
        );
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_scrape_never_fails_because_statistics_are_unavailable(
    pool: sqlx::PgPool,
) -> anyhow::Result<()> {
    init();
    pool.close().await;

    let body = render(&pool).await;

    assert!(
        body.is_some(),
        "a closed database must not take the whole scrape down"
    );
    Ok(())
}

#[test]
fn an_estimated_charge_has_its_own_basis_and_the_token_kinds_stay_apart() {
    use llm_ladder::{Basis, Failure, StepReport, StepResult, Usage};

    init();
    let report = |provider, basis| StepReport {
        ladder: "metrics_test",
        provider,
        result: StepResult::Failed(Failure::Timeout),
        first_output: None,
        elapsed: Duration::from_millis(900),
        usage: Usage {
            input_tokens: 700,
            output_tokens: 9,
            ..Usage::default()
        },
        basis,
        cost_micro_usd: 1_472,
    };
    record_llm_step(&report("guessed", Basis::Estimated));
    record_llm_step(&report("counted", Basis::Reported));

    let Some(handle) = HANDLE.get() else {
        return;
    };
    let rendered = handle.render();
    let line = |metric: &str, provider: &str, more: &str| {
        rendered
            .lines()
            .find(|line| {
                line.starts_with(metric)
                    && line.contains("ladder=\"metrics_test\"")
                    && line.contains(&format!("provider=\"{provider}\""))
                    && line.contains(more)
            })
            .map(str::to_owned)
    };
    let spend = line(llm::SPEND, "guessed", "basis=\"estimated\"").expect("an estimated spend");
    assert!(spend.ends_with(" 1472"), "{spend}");
    assert!(line(llm::SPEND, "counted", "basis=\"reported\"").is_some());
    for kind in ["input", "cache_read", "cache_write", "output"] {
        let kind = format!("kind=\"{kind}\"");
        assert!(line(llm::TOKENS, "guessed", &kind).is_some(), "{kind}");
    }
    let input = line(llm::TOKENS, "guessed", "kind=\"input\"").unwrap();
    assert!(
        input.contains("basis=\"estimated\"") && input.ends_with(" 700"),
        "{input}"
    );
    let steps = line(llm::STEPS, "guessed", "result=\"failed_timeout\"").unwrap();
    assert!(steps.ends_with(" 1"), "{steps}");
}
