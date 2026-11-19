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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_track_owner_may_delete_a_foreign_comment_on_their_track(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = linked(&pool, Graph::Reactions).await?;
    let queue = queue(&pool)?;
    for (track, uploader) in [("200", ACCOUNT), ("201", "7777")] {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms,
                                 uploader_sc_user_id)
             VALUES ($1, $2, 't', 't', 1000, $3)",
        )
        .bind(track)
        .bind(format!("soundcloud:tracks:{track}"))
        .bind(uploader)
        .execute(&pool)
        .await?;
    }
    for (comment, track) in [("60", "200"), ("61", "201")] {
        sqlx::query(
            "INSERT INTO track_comments (id, sc_track_id, sc_comment_id, user_urn, body)
             VALUES ($1, $2, $3, 'soundcloud:users:9999', 'hi')",
        )
        .bind(Uuid::now_v7())
        .bind(track)
        .bind(comment)
        .execute(&pool)
        .await?;
    }

    let on_own_track = fixture.mobile.delete_comment(&queue, ACCOUNT, "60").await?;
    let on_foreign_track = fixture.mobile.delete_comment(&queue, ACCOUNT, "61").await;
    let unknown = fixture.mobile.delete_comment(&queue, ACCOUNT, "62").await?;

    assert_eq!(on_own_track["targetUrn"], "soundcloud:comments:60");
    assert_eq!(
        status_of(on_foreign_track.unwrap_err()),
        StatusCode::FORBIDDEN
    );
    assert_eq!(unknown["targetUrn"], "soundcloud:comments:62");
    let targets: Vec<String> = queued(&pool).await?.into_iter().map(|row| row.1).collect();
    assert_eq!(
        targets,
        vec!["soundcloud:comments:60", "soundcloud:comments:62"]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_second_rejection_marks_the_rejected_credential_for_relink(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = linked(&pool, Graph::Reactions).await?;

    let stale = fixture
        .mobile
        .require_relink(ACCOUNT, "an-older-token")
        .await;
    let after_stale = credential_status(&pool).await?;
    let rejected = fixture
        .mobile
        .require_relink(ACCOUNT, "old-mobile-access")
        .await;
    let after_match = credential_status(&pool).await?;
    let again = fixture
        .mobile
        .require_relink(ACCOUNT, "old-mobile-access")
        .await;

    for error in [stale, rejected, again] {
        let (status, code) = code_of(error).await;
        assert_eq!(
            (status, code.as_str()),
            (StatusCode::CONFLICT, "mobile_relink_required")
        );
    }
    assert_eq!(after_stale, ("active".to_owned(), 0));
    assert_eq!(after_match, ("relink_required".to_owned(), 1));
    assert_eq!(credential_status(&pool).await?, after_match);
    let read = fixture
        .mobile
        .track_reactions(fixture.session_id, ACCOUNT, TRACK, "5")
        .await;
    assert_eq!(status_of(read.unwrap_err()), StatusCode::CONFLICT);
    Ok(())
}

async fn credential_status(pool: &PgPool) -> anyhow::Result<(String, i32)> {
    Ok(sqlx::query_as(
        "SELECT status, refresh_failure_count FROM mobile_credentials
         WHERE soundcloud_user_id = $1",
    )
    .bind(ACCOUNT)
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_unlinked_or_relink_account_cannot_like_a_comment(pool: PgPool) -> anyhow::Result<()> {
    let fixture = fixture_graph(&pool, Token::Pair, Me::Account(ACCOUNT), Graph::Reactions).await?;
    let queue = queue(&pool)?;

    let like = fixture
        .mobile
        .like_comment(&queue, ACCOUNT, "soundcloud:comments:9", Some(TRACK))
        .await;
    let unlike = fixture
        .mobile
        .unlike_comment(&queue, ACCOUNT, "soundcloud:comments:9", Some(TRACK))
        .await;
    for refused in [like, unlike] {
        let (status, code) = code_of(refused.unwrap_err()).await;
        assert_eq!(
            (status, code.as_str()),
            (StatusCode::NOT_FOUND, "mobile_not_linked")
        );
    }

    old_link(&pool).await?;
    sqlx::query("UPDATE mobile_credentials SET status = 'relink_required'")
        .execute(&pool)
        .await?;
    let stale = fixture
        .mobile
        .like_comment(&queue, ACCOUNT, "soundcloud:comments:9", Some(TRACK))
        .await;
    let (status, code) = code_of(stale.unwrap_err()).await;
    assert_eq!(
        (status, code.as_str()),
        (StatusCode::CONFLICT, "mobile_relink_required")
    );
    assert!(queued(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_comment_like_is_queued_with_the_track_from_our_tables_or_the_query(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = linked(&pool, Graph::Reactions).await?;
    let queue = queue(&pool)?;
    sqlx::query(
        "INSERT INTO track_comments (id, sc_track_id, sc_comment_id, user_urn, body)
         VALUES ($1, '123', '70', 'soundcloud:users:9999', 'hi')",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await?;

    let known = fixture
        .mobile
        .like_comment(&queue, ACCOUNT, "70", Some("soundcloud:tracks:555"))
        .await?;
    let asked = fixture
        .mobile
        .like_comment(&queue, ACCOUNT, "71", Some("777"))
        .await?;
    let missing = fixture
        .mobile
        .like_comment(&queue, ACCOUNT, "72", None)
        .await;
    let broken = fixture
        .mobile
        .like_comment(&queue, ACCOUNT, "soundcloud:tracks:7", Some(TRACK))
        .await;
    let bad_track = fixture
        .mobile
        .like_comment(&queue, ACCOUNT, "73", Some("soundcloud:users:1"))
        .await;

    assert_eq!(known["actionType"], "comment_like");
    assert_eq!(known["targetUrn"], "soundcloud:comments:70");
    assert_eq!(asked["targetUrn"], "soundcloud:comments:71");
    let (status, code) = code_of(missing.unwrap_err()).await;
    assert_eq!(
        (status, code.as_str()),
        (StatusCode::BAD_REQUEST, "track_urn_required")
    );
    assert_eq!(status_of(broken.unwrap_err()), StatusCode::BAD_REQUEST);
    assert_eq!(status_of(bad_track.unwrap_err()), StatusCode::BAD_REQUEST);
    assert_eq!(
        queued(&pool).await?,
        vec![
            (
                "comment_like".to_owned(),
                "soundcloud:comments:70".to_owned(),
                Some(json!({"parent_urn": "soundcloud:tracks:123"})),
                false
            ),
            (
                "comment_like".to_owned(),
                "soundcloud:comments:71".to_owned(),
                Some(json!({"parent_urn": "soundcloud:tracks:777"})),
                false
            ),
        ]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_unlike_annuls_the_queued_like_of_the_same_comment(pool: PgPool) -> anyhow::Result<()> {
    let fixture = linked(&pool, Graph::Reactions).await?;
    let queue = queue(&pool)?;

    fixture
        .mobile
        .like_comment(&queue, ACCOUNT, "80", Some(TRACK))
        .await?;
    let unlike = fixture
        .mobile
        .unlike_comment(&queue, ACCOUNT, "80", Some(TRACK))
        .await?;

    assert_eq!(unlike["actionType"], "comment_unlike");
    let rows = queued(&pool).await?;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.3));
    Ok(())
}
