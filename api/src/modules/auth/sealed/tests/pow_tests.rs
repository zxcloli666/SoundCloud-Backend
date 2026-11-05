use axum::Router;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use sqlx::PgPool;

use super::support::{
    Device, PEER, app, cfg, exchange, from, login, marker, oauth_app, seal, sealed, send, solve,
    with_pow,
};
use crate::modules::auth::sealed::Sealed;
use crate::modules::auth::sealed::seed::epoch_now;
use std::sync::Arc;

const LOGIN: &str = "/auth/login";

fn tight() -> crate::modules::auth::sealed::config::SealedCfg {
    let mut config = cfg();
    config.free_per_min = 1;
    config
}

async fn outcome(router: &Router, device: &Device, pow: sc_chat_wire::proto::Pow) -> (u32, Value) {
    send(router, with_pow(login(device), pow), device).await
}

fn seed_of(body: &Value) -> Vec<u8> {
    URL_SAFE_NO_PAD
        .decode(body["seed"].as_str().expect("a seed"))
        .expect("a base64url seed")
}

fn current(node: &Arc<Sealed>) -> [u8; 32] {
    node.seeds.seed(epoch_now())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_full_free_lane_sends_a_marker_to_pow_and_a_solution_passes_once(
    pool: PgPool,
) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let node = sealed(tight(), &pool)?;
    let router = app(&node);
    let device = Device::new();

    assert_eq!(send(&router, login(&device), &device).await.0, 200);
    let (status, busy) = send(&router, login(&device), &device).await;
    assert_eq!((status, busy["error"].as_str()), (503, Some("chat_busy")));

    let (status, required) = send(&router, marker(login(&device)), &device).await;
    assert_eq!(status, 429, "{required}");
    assert_eq!(required["error"], "chat_pow_required");
    assert_eq!(required["v"], 1);
    assert_eq!(required["effort"], 1);
    assert_eq!(seed_of(&required), current(&node));

    let pow = solve(&seed_of(&required), 1, &device.auth_pub(), LOGIN);
    let (status, started) = outcome(&router, &device, pow.clone()).await;
    assert_eq!(status, 200, "{started}");

    let (status, reused) = outcome(&router, &device, pow).await;
    assert_eq!(
        (status, reused["error"].as_str()),
        (409, Some("chat_replay"))
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_replayed_pow_envelope_is_refused_by_the_solution_claim(
    pool: PgPool,
) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let node = sealed(tight(), &pool)?;
    let router = app(&node);
    let device = Device::new();
    let pow = solve(&current(&node), 1, &device.auth_pub(), LOGIN);
    let request = seal(with_pow(login(&device), pow), &device);

    assert_eq!(exchange(&router, &request, from(PEER)).await.0, 200);
    let (status, body) = exchange(&router, &request, from(PEER)).await;
    assert_eq!((status, body["error"].as_str()), (409, Some("chat_replay")));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_seed_of_another_node_is_asked_again_and_still_works_at_home(
    pool: PgPool,
) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let main = sealed(cfg(), &pool)?;
    let mut other = cfg();
    other.node = "star".to_owned();
    let star = sealed(other, &pool)?;
    let device = Device::new();
    let pow = solve(&current(&main), 1, &device.auth_pub(), LOGIN);

    let (status, again) = outcome(&app(&star), &device, pow.clone()).await;
    assert_eq!(
        (status, again["error"].as_str()),
        (429, Some("chat_pow_required"))
    );
    assert_eq!(seed_of(&again), current(&star));
    assert_ne!(current(&star), current(&main));

    assert_eq!(outcome(&app(&main), &device, pow).await.0, 200);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_previous_epoch_passes_and_two_epochs_old_is_asked_again(
    pool: PgPool,
) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let node = sealed(cfg(), &pool)?;
    let router = app(&node);
    let device = Device::new();
    let (epoch, old, previous) = loop {
        let epoch = epoch_now();
        let old = solve(&node.seeds.seed(epoch - 2), 1, &device.auth_pub(), LOGIN);
        let previous = solve(&node.seeds.seed(epoch - 1), 1, &device.auth_pub(), LOGIN);
        if epoch_now() == epoch {
            break (epoch, old, previous);
        }
    };
    assert_eq!(epoch, epoch_now());

    let (status, body) = outcome(&router, &device, old).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (429, Some("chat_pow_required"))
    );
    assert_eq!(outcome(&router, &device, previous).await.0, 200);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_solution_for_another_path_is_invalid(pool: PgPool) -> anyhow::Result<()> {
    let node = sealed(cfg(), &pool)?;
    let router = app(&node);
    let device = Device::new();
    let pow = solve(&current(&node), 1, &device.auth_pub(), "/auth/link/create");

    let (status, body) = outcome(&router, &device, pow).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("chat_invalid"))
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_effort_below_the_required_one_is_asked_again(pool: PgPool) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let node = sealed(cfg(), &pool)?;
    let router = app(&node);
    let device = Device::new();
    let low = solve(&current(&node), 1, &device.auth_pub(), LOGIN);
    let enough = solve(&current(&node), 2, &device.auth_pub(), LOGIN);
    let effort = &node.gates.login.effort;
    for _ in 0..600 {
        effort.note();
    }
    effort.tick();
    effort.tick();
    assert_eq!((effort.current(), effort.minimum()), (4, 2));
    *node.gates.login.ticked.lock().expect("a clock") = std::time::Instant::now();

    let (status, body) = outcome(&router, &device, low).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (429, Some("chat_pow_required"))
    );
    assert_eq!(body["effort"], 4);

    assert_eq!(outcome(&router, &device, enough).await.0, 200);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_full_pow_lane_asks_for_pow_again(pool: PgPool) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let mut config = cfg();
    config.pow_per_min = 1;
    let node = sealed(config, &pool)?;
    let router = app(&node);
    let first = Device::new();
    let second = Device::new();

    let pow = solve(&current(&node), 1, &first.auth_pub(), LOGIN);
    assert_eq!(outcome(&router, &first, pow).await.0, 200);

    let pow = solve(&current(&node), 1, &second.auth_pub(), LOGIN);
    let (status, body) = outcome(&router, &second, pow).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (429, Some("chat_pow_required"))
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_client_over_its_own_free_limit_is_sent_to_pow_while_others_pass(
    pool: PgPool,
) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let node = sealed(cfg(), &pool)?;
    let router = app(&node);
    let device = Device::new();

    for _ in 0..15 {
        assert_eq!(send(&router, login(&device), &device).await.0, 200);
    }
    let (status, busy) = send(&router, login(&device), &device).await;
    assert_eq!((status, busy["error"].as_str()), (503, Some("chat_busy")));
    let (status, required) = send(&router, marker(login(&device)), &device).await;
    assert_eq!(
        (status, required["error"].as_str()),
        (429, Some("chat_pow_required"))
    );

    let neighbour = seal(login(&device), &device);
    let (status, _) = exchange(&router, &neighbour, from([203, 0, 113, 9])).await;
    assert_eq!(status, 200);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn with_pow_off_a_marker_gets_busy_and_a_solution_is_ignored(
    pool: PgPool,
) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let mut config = tight();
    config.pow = false;
    let node = sealed(config, &pool)?;
    let router = app(&node);
    let device = Device::new();

    assert_eq!(send(&router, login(&device), &device).await.0, 200);
    let (status, body) = send(&router, marker(login(&device)), &device).await;
    assert_eq!((status, body["error"].as_str()), (503, Some("chat_busy")));
    Ok(())
}
