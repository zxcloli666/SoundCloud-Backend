use chrono::{DateTime, Utc};
use serde_json::json;
use uuid::Uuid;
use write_through as wt;
use wt::{ConnectionDropOrphan, MobileCredentialDrop, MobileCredentialLink};

use crate::harness::Pair;

const LINKED: &str = "INSERT INTO public.oauth_apps (id, name, client_id, client_secret, redirect_uri, client_kind, app_id, app_version, user_agent) VALUES
    ('40000000-0000-4000-8000-000000000005', 'mobile', 'client', 'secret', 'sc://auth', 'mobile', 'fake.app', '1.0.0', 'FakeAgent/1.0');
    INSERT INTO public.mobile_credentials (soundcloud_user_id, oauth_app_id, device_id, access_token, refresh_token,
        expires_at, scope, linked_at, status, refresh_lease_id, refresh_lease_expires_at, last_refresh_error_kind) VALUES
    ('31', '40000000-0000-4000-8000-000000000005', '50000000-0000-4000-8000-000000000001', 'm1', 'mr1',
     '2026-11-01 00:00:00+00', '*', '2026-10-30 00:00:00+00', 'relink_required', 'aaaaaaaa-0000-4000-8000-000000000031',
     '2026-10-30 10:01:00+00', 'relink_required'),
    ('32', '40000000-0000-4000-8000-000000000005', '50000000-0000-4000-8000-000000000002', 'm2', 'mr2',
     '2026-11-01 00:00:00+00', '*', '2026-10-30 00:00:00+00', 'active', NULL, NULL, NULL);
    INSERT INTO public.soundcloud_connections (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope) VALUES
    ('20000000-0000-4000-8000-000000000011', '31', 'alone', 'a1', 'r1', '2026-11-01 00:00:00+00', ''),
    ('20000000-0000-4000-8000-000000000012', '32', 'twice', 'a2', 'r2', '2026-11-01 00:00:00+00', ''),
    ('20000000-0000-4000-8000-000000000013', '32', 'twice', 'a3', 'r3', '2026-11-01 00:00:00+00', '');
    INSERT INTO public.sessions (id, soundcloud_connection_id) VALUES
    ('30000000-0000-4000-8000-000000000013', '20000000-0000-4000-8000-000000000013')";

fn at(stamp: &str) -> DateTime<Utc> {
    stamp.parse().unwrap()
}

fn link(version: i64, pair: &'static str) -> MobileCredentialLink<'static> {
    MobileCredentialLink {
        soundcloud_user_id: "31",
        oauth_app_id: "40000000-0000-4000-8000-000000000005".parse().unwrap(),
        device_id: "50000000-0000-4000-8000-000000000009".parse().unwrap(),
        access_token: pair,
        refresh_token: pair,
        expires_at: at("2026-11-06T10:00:00Z"),
        scope: "*",
        link_v: version,
        at: at("2026-10-30T10:00:00Z"),
    }
}

async fn link_row(pool: &sqlx::PgPool) -> (String, i64, i64, String, Option<Uuid>, Uuid) {
    sqlx::query_as(
        "SELECT access_token, link_v, token_v, status, refresh_lease_id, device_id
         FROM public.mobile_credentials WHERE soundcloud_user_id = '31'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn accounts(pool: &sqlx::PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT soundcloud_user_id FROM public.mobile_credentials ORDER BY 1")
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn plan_seen_after_the_link(pool: &sqlx::PgPool, account: &str) {
    let newer: bool = sqlx::query_scalar(
        "UPDATE public.mobile_credentials SET plan = 'none', plan_seen_at = now()
         WHERE soundcloud_user_id = $1 RETURNING plan_v > token_v",
    )
    .bind(account)
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(newer);
}

async fn receipts(pool: &sqlx::PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT pk ->> 'soundcloud_user_id' FROM public.repl_purges
         WHERE tbl = 'mobile_credentials' ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_relink_lands_the_same_on_both_nodes_and_an_older_one_only_touches_the_row() {
    let pair = Pair::new(LINKED).await;
    let older: i64 = sqlx::query_scalar("SELECT public.scd_clock()")
        .fetch_one(&pair.api)
        .await
        .unwrap();
    let newer: i64 = sqlx::query_scalar(
        "SELECT public.scd_hlc(token_v) FROM public.mobile_credentials WHERE soundcloud_user_id = '31'",
    )
    .fetch_one(&pair.api)
    .await
    .unwrap();

    let run = pair.run(&link(newer, "fresh"), &[]).await;
    assert_eq!(run.assert_same(), json!(true));
    let landed = link_row(&pair.core).await;
    assert_eq!(
        landed,
        (
            "fresh".to_owned(),
            newer,
            newer,
            "active".to_owned(),
            None,
            "50000000-0000-4000-8000-000000000009".parse().unwrap()
        )
    );
    assert_eq!(link_row(&pair.api).await, landed);

    let run = pair.run(&link(older, "late"), &[]).await;
    assert_eq!(run.assert_same(), json!(false));
    assert_eq!(link_row(&pair.core).await, landed);
    assert_eq!(link_row(&pair.api).await, landed);
    pair.finish().await;
}

#[tokio::test]
async fn an_unlink_purges_on_core_with_a_receipt_and_a_missing_link_is_a_no_op() {
    let pair = Pair::new(LINKED).await;
    plan_seen_after_the_link(&pair.core, "32").await;

    let run = pair
        .run(
            &MobileCredentialDrop {
                soundcloud_user_id: "32",
            },
            &[],
        )
        .await;
    assert_eq!(run.core, json!(true));
    run.assert_same();
    let run = pair
        .run(
            &MobileCredentialDrop {
                soundcloud_user_id: "99",
            },
            &[],
        )
        .await;
    assert_eq!(run.core, json!(false));
    run.assert_same();

    assert_eq!(accounts(&pair.core).await, vec!["31"]);
    assert_eq!(receipts(&pair.core).await, vec!["32"]);
    pair.finish().await;
}

#[tokio::test]
async fn an_orphan_drop_takes_the_mobile_link_only_with_the_last_connection() {
    let pair = Pair::new(LINKED).await;
    plan_seen_after_the_link(&pair.core, "31").await;

    let alone = ConnectionDropOrphan {
        connection_id: "20000000-0000-4000-8000-000000000011".parse().unwrap(),
    };
    let run = pair.run(&alone, &[]).await;
    assert_eq!(run.core, json!(true));
    run.assert_same();
    let twice = ConnectionDropOrphan {
        connection_id: "20000000-0000-4000-8000-000000000012".parse().unwrap(),
    };
    let run = pair.run(&twice, &[]).await;
    assert_eq!(run.core, json!(true));
    run.assert_same();

    assert_eq!(accounts(&pair.core).await, vec!["32"]);
    assert_eq!(receipts(&pair.core).await, vec!["31"]);
    pair.finish().await;
}
