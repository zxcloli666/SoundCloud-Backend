use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::tests::{ACCOUNT, Graph, Me, Token, fixture_graph, old_link, status_of};
use crate::error::AppError;
use crate::modules::sync_queue::SyncQueueService;

const TRACK: &str = "soundcloud:tracks:123";
const FIRE: &str = "sc:interactiontypevalue:1f525";

type Queued = (String, String, Option<Value>, bool);

fn queue(pool: &PgPool) -> anyhow::Result<std::sync::Arc<SyncQueueService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(SyncQueueService::new(pool.clone(), redis))
}

async fn linked(pool: &PgPool, graph: Graph) -> anyhow::Result<super::tests::Fixture> {
    let fixture = fixture_graph(pool, Token::Pair, Me::Account(ACCOUNT), graph).await?;
    old_link(pool).await?;
    Ok(fixture)
}

async fn queued(pool: &PgPool) -> anyhow::Result<Vec<Queued>> {
    Ok(sqlx::query_as(
        "SELECT action_type, target_urn, payload, cancelled FROM sync_queue ORDER BY action_type",
    )
    .fetch_all(pool)
    .await?)
}

async fn code_of(error: AppError) -> (StatusCode, String) {
    let response = error.into_response();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let code = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|body| {
            ["code", "error"]
                .iter()
                .find_map(|key| body.get(*key).and_then(Value::as_str).map(str::to_owned))
        })
        .unwrap_or_default();
    (status, code)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_unlinked_account_is_refused_at_once_and_nothing_is_queued(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = fixture_graph(&pool, Token::Pair, Me::Account(ACCOUNT), Graph::Reactions).await?;
    let queue = queue(&pool)?;

    let reaction = fixture
        .mobile
        .set_reaction(&queue, ACCOUNT, TRACK, 5, None)
        .await;
    let removal = fixture
        .mobile
        .remove_reaction(&queue, ACCOUNT, TRACK, 5, None)
        .await;
    let comment = fixture
        .mobile
        .delete_comment(&queue, ACCOUNT, "soundcloud:comments:9")
        .await;
    let read = fixture
        .mobile
        .track_reactions(fixture.session_id, ACCOUNT, TRACK, "5")
        .await;

    for refused in [reaction, removal, comment] {
        let (status, code) = code_of(refused.unwrap_err()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(code, "mobile_not_linked");
    }
    let (status, code) = code_of(read.unwrap_err()).await;
    assert_eq!(
        (status, code.as_str()),
        (StatusCode::NOT_FOUND, "mobile_not_linked")
    );
    assert!(queued(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_relink_required_account_is_refused_with_its_own_code(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = linked(&pool, Graph::Reactions).await?;
    sqlx::query("UPDATE mobile_credentials SET status = 'relink_required'")
        .execute(&pool)
        .await?;
    let queue = queue(&pool)?;

    let reaction = fixture
        .mobile
        .set_reaction(&queue, ACCOUNT, TRACK, 5, None)
        .await;
    let read = fixture
        .mobile
        .track_reactions(fixture.session_id, ACCOUNT, TRACK, "5")
        .await;

    for refused in [reaction.map(|_| ()), read.map(|_| ())] {
        let (status, code) = code_of(refused.unwrap_err()).await;
        assert_eq!(
            (status, code.as_str()),
            (StatusCode::CONFLICT, "mobile_relink_required")
        );
    }
    assert!(queued(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_reaction_is_queued_on_its_moment_with_the_default_or_the_chosen_value(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = linked(&pool, Graph::Reactions).await?;
    let queue = queue(&pool)?;

    let first = fixture
        .mobile
        .set_reaction(&queue, ACCOUNT, "123", 45, None)
        .await?;
    fixture
        .mobile
        .set_reaction(
            &queue,
            ACCOUNT,
            TRACK,
            46,
            Some("sc:interactiontypevalue:1f60d"),
        )
        .await?;

    assert_eq!(
        first,
        json!({
            "status": "queued",
            "actionType": "track_reaction_set",
            "targetUrn": "soundcloud:tracks:123:moments:45"
        })
    );
    assert_eq!(
        queued(&pool).await?,
        vec![
            (
                "track_reaction_set".to_owned(),
                "soundcloud:tracks:123:moments:45".to_owned(),
                None,
                false
            ),
            (
                "track_reaction_set".to_owned(),
                "soundcloud:tracks:123:moments:46".to_owned(),
                Some(json!({ "value_urn": "sc:interactiontypevalue:1f60d" })),
                false
            ),
        ]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_removal_before_the_set_left_jobs_cancels_it(pool: PgPool) -> anyhow::Result<()> {
    let fixture = linked(&pool, Graph::Reactions).await?;
    let queue = queue(&pool)?;

    fixture
        .mobile
        .set_reaction(&queue, ACCOUNT, TRACK, 5, Some(FIRE))
        .await?;
    fixture
        .mobile
        .remove_reaction(&queue, ACCOUNT, TRACK, 5, Some(FIRE))
        .await?;

    let rows = queued(&pool).await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .map(|row| (row.0.as_str(), row.3))
            .collect::<Vec<_>>(),
        vec![
            ("track_reaction_remove", true),
            ("track_reaction_set", true)
        ]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_bad_track_second_or_value_is_a_plain_bad_request(pool: PgPool) -> anyhow::Result<()> {
    let fixture = linked(&pool, Graph::Reactions).await?;
    let queue = queue(&pool)?;

    let cases = [
        ("soundcloud:users:1", 5, None, "invalid_track_urn"),
        ("12x", 5, None, "invalid_track_urn"),
        (TRACK, 86_401, None, "invalid_reaction_second"),
        (TRACK, 5, Some("1f525"), "invalid_reaction_value"),
        (
            TRACK,
            5,
            Some("sc:interactiontypevalue:"),
            "invalid_reaction_value",
        ),
        (
            TRACK,
            5,
            Some("sc:interactiontypevalue:a b"),
            "invalid_reaction_value",
        ),
    ];
    for (track, second, value, expected) in cases {
        let refused = fixture
            .mobile
            .set_reaction(&queue, ACCOUNT, track, second, value)
            .await;
        let (status, code) = code_of(refused.unwrap_err()).await;
        assert_eq!((status, code.as_str()), (StatusCode::BAD_REQUEST, expected));
    }
    assert!(queued(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn only_an_own_comment_is_queued_for_deletion(pool: PgPool) -> anyhow::Result<()> {
    let fixture = linked(&pool, Graph::Reactions).await?;
    let queue = queue(&pool)?;
    for (comment, author) in [("55", ACCOUNT), ("56", "9999")] {
        sqlx::query(
            "INSERT INTO track_comments (id, sc_track_id, sc_comment_id, user_urn, body)
             VALUES ($1, '123', $2, $3, 'hi')",
        )
        .bind(Uuid::now_v7())
        .bind(comment)
        .bind(format!("soundcloud:users:{author}"))
        .execute(&pool)
        .await?;
    }

    let foreign = fixture
        .mobile
        .delete_comment(&queue, ACCOUNT, "soundcloud:comments:56")
        .await;
    let own = fixture.mobile.delete_comment(&queue, ACCOUNT, "55").await?;
    let broken = fixture.mobile.delete_comment(&queue, ACCOUNT, "0").await;

    assert_eq!(status_of(foreign.unwrap_err()), StatusCode::FORBIDDEN);
    assert_eq!(status_of(broken.unwrap_err()), StatusCode::BAD_REQUEST);
    assert_eq!(own["targetUrn"], "soundcloud:comments:55");
    assert_eq!(
        queued(&pool).await?,
        vec![(
            "comment_delete".to_owned(),
            "soundcloud:comments:55".to_owned(),
            None,
            false
        )]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn reactions_are_read_with_the_owner_token_and_mapped_per_moment(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = linked(&pool, Graph::Reactions).await?;

    let read = fixture
        .mobile
        .track_reactions(fixture.session_id, ACCOUNT, TRACK, "5,6")
        .await?;

    assert_eq!(read.track_urn, TRACK);
    assert_eq!(read.moments.len(), 2);
    assert_eq!(read.moments[0].total, 3);
    assert_eq!(read.moments[0].own.as_deref(), Some(FIRE));
    assert_eq!(read.moments[1].total, 0);
    assert_eq!(read.moments[1].own, None);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_bot_check_and_a_dead_token_are_plain_errors_without_a_workaround(
    pool: PgPool,
) -> anyhow::Result<()> {
    let checked = linked(&pool, Graph::Captcha).await?;

    let captcha = checked
        .mobile
        .track_reactions(checked.session_id, ACCOUNT, TRACK, "5")
        .await;

    let (status, code) = code_of(captcha.unwrap_err()).await;
    assert_eq!(
        (status, code.as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "mobile_captcha")
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_rejected_token_that_cannot_be_refreshed_is_unavailable_not_relinked(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = linked(&pool, Graph::Unauthorized).await?;

    let read = fixture
        .mobile
        .track_reactions(fixture.session_id, ACCOUNT, TRACK, "5")
        .await;

    let (status, code) = code_of(read.unwrap_err()).await;
    assert_eq!(
        (status, code.as_str()),
        (StatusCode::BAD_GATEWAY, "mobile_unavailable")
    );
    Ok(())
}
