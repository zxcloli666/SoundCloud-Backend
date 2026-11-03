use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgDatabaseError;

use crate::harness::Pair;
use crate::race::{Holder, ID, OTHER_ID, URN, insert_holder, race, rows};

async fn ship(pool: PgPool, command: &'static str, hold: bool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await?;
    if hold {
        sqlx::query("SET LOCAL wt.hold = 'on'")
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("INSERT INTO public.cmd_outbox (node, id, kind, args, touched) VALUES ('main', $1::uuid, 'test.race', $2, '[]')")
        .bind(command)
        .bind(json!({"id": ID, "urn": URN, "note": "kind"}))
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

async fn verdict(pool: &PgPool, command: &str) -> (Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT (SELECT status FROM public.cmd_done WHERE node = 'main' AND id = $1::uuid),
                (SELECT sqlstate FROM public.cmd_dead WHERE node = 'main' AND id = $1::uuid)",
    )
    .bind(command)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn sqlstate(error: &sqlx::Error) -> (String, Option<String>) {
    let pg = error
        .as_database_error()
        .and_then(|e| e.try_downcast_ref::<PgDatabaseError>())
        .expect("database error");
    (pg.code().to_owned(), pg.hint().map(str::to_owned))
}

#[tokio::test]
async fn dispatch_on_core_hands_a_raced_command_back_to_the_apply_retry() {
    let pair = Pair::new("").await;
    for (command, applied) in [
        ("00000000-0000-0000-0000-0000000000d1", false),
        ("00000000-0000-0000-0000-0000000000d2", true),
    ] {
        sqlx::query("DELETE FROM wt_test.race")
            .execute(&pair.core)
            .await
            .unwrap();
        let first = race(
            &pair.core_db,
            Holder { id: ID, applied },
            ship(pair.core.clone(), command, true),
        )
        .await;
        assert_eq!(
            sqlstate(&first.unwrap_err()),
            ("23505".to_owned(), None),
            "applied holder: {applied}"
        );
        assert_eq!(
            verdict(&pair.core, command).await,
            (None, None),
            "applied holder: {applied}"
        );
        ship(pair.core.clone(), command, false).await.unwrap();
        assert_eq!(
            verdict(&pair.core, command).await,
            (Some("ok".to_owned()), None),
            "applied holder: {applied}"
        );
        assert_eq!(
            rows(&pair.core).await,
            json!([{"id": ID, "note": "kind"}]),
            "applied holder: {applied}"
        );
    }
    pair.finish().await;
}

#[tokio::test]
async fn dispatch_on_core_dead_letters_a_real_violation() {
    let pair = Pair::new("").await;
    let command = "00000000-0000-0000-0000-0000000000d3";
    insert_holder(
        &pair.core_db,
        &Holder {
            id: OTHER_ID,
            applied: false,
        },
    )
    .await;
    ship(pair.core.clone(), command, false).await.unwrap();
    assert_eq!(
        verdict(&pair.core, command).await,
        (Some("dead".to_owned()), Some("23505".to_owned()))
    );
    assert_eq!(
        rows(&pair.core).await,
        json!([{"id": OTHER_ID, "note": "holder"}])
    );
    pair.finish().await;
}
