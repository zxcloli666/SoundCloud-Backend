use chrono::{NaiveDate, TimeZone, Utc};
use sqlx::PgPool;

use super::super::budget;
use super::super::director::{DirectorError, Spend};
use super::{PLAN_USD, TRACK, bench, config, plan, row, star, track};

async fn waiting(pool: &PgPool) {
    star(pool, "12").await;
    track(pool, TRACK, "12").await;
}

async fn spend(pool: &PgPool, period: NaiveDate, usd: f64) {
    let call = Spend {
        call: "plan",
        provider: "opus".to_owned(),
        model: "example-model".to_owned(),
        input_tokens: 10,
        output_tokens: 10,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        usd,
    };
    budget::record(pool, period, "1", &[call]).await.unwrap();
}

async fn pause_reason(pool: &PgPool) -> Option<String> {
    budget::paused(pool)
        .await
        .unwrap()
        .map(|pause| pause.reason)
}

async fn pause_due(pool: &PgPool) {
    sqlx::query("UPDATE video_gen_pause SET paused_until = now()")
        .execute(pool)
        .await
        .unwrap();
    super::due_now(pool).await;
}

#[test]
fn the_month_starts_on_the_reset_day() {
    let before = Utc.with_ymd_and_hms(2026, 11, 16, 23, 0, 0).unwrap();
    let on = Utc.with_ymd_and_hms(2026, 11, 17, 0, 0, 0).unwrap();

    assert_eq!(
        budget::period(before, 17),
        NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()
    );
    assert_eq!(
        budget::period(on, 17),
        NaiveDate::from_ymd_opt(2026, 11, 17).unwrap()
    );
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_ledger_sums_one_period(pool: PgPool) {
    let now = budget::period(Utc::now(), 17);
    let old = NaiveDate::from_ymd_opt(2020, 1, 17).unwrap();
    spend(&pool, now, 1.25).await;
    spend(&pool, now, 0.000_5).await;
    spend(&pool, old, 40.0).await;

    assert!((budget::spent(&pool, now).await.unwrap() - 1.250_5).abs() < 1e-9);
    assert!((budget::spent(&pool, old).await.unwrap() - 40.0).abs() < 1e-9);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_spent_budget_pauses_once_and_resumes_when_there_is_room(pool: PgPool) {
    waiting(&pool).await;
    spend(&pool, budget::period(Utc::now(), 17), 50.0).await;
    let (bench, director, _, wire, inbox) = bench(&pool, config());

    bench.dispatch().await.unwrap();
    bench.dispatch().await.unwrap();
    pause_due(&pool).await;
    bench.dispatch().await.unwrap();

    assert_eq!(inbox.count(), 1);
    assert!(inbox.messages.lock().unwrap()[0].contains("$50.00"));
    assert_eq!(pause_reason(&pool).await.as_deref(), Some(budget::BUDGET));
    assert!(director.briefs.lock().unwrap().is_empty());
    assert!(wire.sent.lock().unwrap().is_empty());

    let mut more = config();
    more.month_usd = 80.0;
    let (bench, director, _, wire, inbox) = super::bench(&pool, more);
    director.plans(Ok(plan()));
    pause_due(&pool).await;
    bench.dispatch().await.unwrap();

    assert_eq!(pause_reason(&pool).await, None);
    assert_eq!(inbox.count(), 0);
    assert_eq!(wire.subjects(), ["video.keyframe.new"]);
    assert_eq!(row(&pool, TRACK).await.0, "keyframe");
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_down_api_pauses_once_and_the_next_answer_resumes(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, inbox) = bench(&pool, config());
    director.plans(Err(DirectorError::Down("503".to_owned())));
    director.plans(Err(DirectorError::Down("503".to_owned())));
    director.plans(Ok(plan()));

    bench.dispatch().await.unwrap();
    bench.dispatch().await.unwrap();

    assert_eq!(inbox.count(), 1);
    assert_eq!(pause_reason(&pool).await.as_deref(), Some(budget::API_DOWN));
    assert_eq!(director.briefs.lock().unwrap().len(), 1);

    pause_due(&pool).await;
    bench.dispatch().await.unwrap();

    assert_eq!(inbox.count(), 1);
    assert_eq!(pause_reason(&pool).await.as_deref(), Some(budget::API_DOWN));

    pause_due(&pool).await;
    bench.dispatch().await.unwrap();

    assert_eq!(pause_reason(&pool).await, None);
    assert_eq!(wire.subjects(), ["video.keyframe.new"]);
    let spent = budget::spent(&pool, budget::period(Utc::now(), 17))
        .await
        .unwrap();
    assert!((spent - PLAN_USD).abs() < 1e-9);
}
