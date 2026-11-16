use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::handlers::enrich::resolver::test_support::ctx;
use crate::llm::test_support::{FIRST, SECOND, Scripted, refine_ladder};

fn resolver(pool: &PgPool, worker: &Arc<Scripted>) -> anyhow::Result<AiResolverClient> {
    let config = AiConfig {
        refine: None,
        timeout_ms: 20_000,
        daily_budget: 1,
    };
    Ok(AiResolverClient::new(
        refine_ladder(worker.clone())?,
        Arc::new(Normalizer::new()?),
        pool.clone(),
        &config,
    ))
}

fn unsure() -> TrackContext {
    ctx("Kendrick Lamar & SZA - luther", Some("TDE"), None)
}

fn answer(primary: &str) -> serde_json::Value {
    json!({
        "primary_artist": primary,
        "featured": ["SZA"],
        "producers": [],
        "remixers": [],
        "album": null,
        "confidence": 0.93,
    })
}

async fn cached(pool: &PgPool, track: &TrackContext) -> Option<ResolveArtistData> {
    AiStore::new(pool.clone(), 1)
        .cached(&format!("resolve:{}", context_hash(track)))
        .await
}

async fn budget_left(pool: &PgPool) -> bool {
    AiStore::new(pool.clone(), 1).take_budget().await
}

fn primary(result: &ResolveResult) -> Option<&str> {
    result.primary.first().map(|artist| artist.name.as_str())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_confident_parse_never_asks_the_model(pool: PgPool) -> anyhow::Result<()> {
    let worker = Arc::new(Scripted::default());

    let result = resolver(&pool, &worker)?
        .resolve(&ctx("x", None, Some("Artist")))
        .await;

    assert!(result.is_none());
    assert!(worker.asked().is_empty());
    assert!(budget_left(&pool).await);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_ungrounded_answer_moves_on_to_the_next_step(pool: PgPool) -> anyhow::Result<()> {
    let worker = Arc::new(Scripted::default());
    worker.answer(FIRST, answer("Drake"));
    worker.answer(SECOND, answer("KENDRICK LAMAR"));
    let track = unsure();

    let result = resolver(&pool, &worker)?
        .resolve(&track)
        .await
        .ok_or_else(|| anyhow::anyhow!("no answer"))?;

    assert_eq!(result.source, ResolveSource::Ai);
    assert_eq!(primary(&result), Some("Kendrick Lamar"));
    assert_eq!(result.confidence, 0.75);
    let subjects: Vec<String> = worker
        .asked()
        .into_iter()
        .map(|(subject, _)| subject)
        .collect();
    assert_eq!(subjects, [FIRST, SECOND]);
    let stored = cached(&pool, &track)
        .await
        .ok_or_else(|| anyhow::anyhow!("the answer was not cached"))?;
    assert_eq!(stored.primary_artist.as_deref(), Some("Kendrick Lamar"));
    assert_eq!(stored.source, RpcSource::Llm);
    assert!(!budget_left(&pool).await);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_ladder_without_an_answer_leaves_the_heuristic_and_refunds(
    pool: PgPool,
) -> anyhow::Result<()> {
    let worker = Arc::new(Scripted::default());
    worker.answer(FIRST, answer("Drake"));
    let track = unsure();

    let result = resolver(&pool, &worker)?.resolve(&track).await;

    assert!(result.is_none());
    assert_eq!(worker.asked().len(), 2);
    assert!(cached(&pool, &track).await.is_none());
    assert!(budget_left(&pool).await);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_exhausted_daily_budget_asks_nobody(pool: PgPool) -> anyhow::Result<()> {
    let worker = Arc::new(Scripted::default());
    worker.answer(FIRST, answer("Kendrick Lamar"));
    assert!(budget_left(&pool).await);

    let result = resolver(&pool, &worker)?.resolve(&unsure()).await;

    assert!(result.is_none());
    assert!(worker.asked().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn cached_worker_replies_are_still_read(pool: PgPool) -> anyhow::Result<()> {
    let worker = Arc::new(Scripted::default());
    let store = AiStore::new(pool.clone(), 1);
    let model = unsure();
    let parsed = ctx("рассвет", Some("Chill Beats Radio"), None);
    let llm: ResolveArtistData = serde_json::from_value(json!({
        "primary_artist": "Kendrick Lamar",
        "featured": [],
        "producers": [],
        "remixers": [],
        "album": {"title": "GNX"},
        "confidence": 0.6,
        "source": "llm",
    }))?;
    let deterministic: ResolveArtistData = serde_json::from_value(json!({
        "primary_artist": null,
        "featured": [],
        "producers": [],
        "remixers": [],
        "album": null,
        "confidence": 0.3,
        "source": "deterministic",
    }))?;
    store
        .remember(&format!("resolve:{}", context_hash(&model)), &llm)
        .await;
    store
        .remember(
            &format!("resolve:{}", context_hash(&parsed)),
            &deterministic,
        )
        .await;
    let client = resolver(&pool, &worker)?;

    let from_model = client
        .resolve(&model)
        .await
        .ok_or_else(|| anyhow::anyhow!("the cached answer was not read"))?;
    let from_parser = client.resolve(&parsed).await;

    assert_eq!(primary(&from_model), Some("Kendrick Lamar"));
    assert_eq!(
        from_model.album.map(|album| album.title),
        Some("GNX".to_owned())
    );
    assert!(from_parser.is_none());
    assert!(worker.asked().is_empty());
    assert!(budget_left(&pool).await);
    Ok(())
}
