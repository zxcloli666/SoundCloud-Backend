use std::sync::Arc;

use chat_wire::bind::make_bind;
use chat_wire::prost::Message;
use chat_wire::random_bytes;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::auth::model::LoginRequest;
use crate::modules::auth::service::login::LoginIdentity;
use crate::modules::chat::ChatState;
use crate::modules::chat::caller::CallerBind;
use crate::modules::chat::test_support::chat_state;
use crate::sc::ScTokenResponse;

fn state(pool: &PgPool) -> anyhow::Result<Arc<ChatState>> {
    let redis = crate::redis::connect_url("redis://127.0.0.1:1")?;
    Ok(chat_state(pool.clone(), redis, true))
}

fn bind() -> Vec<u8> {
    let device = Uuid::new_v4().to_string();
    let seed: [u8; 32] = random_bytes().expect("randomness");
    let box_pub: [u8; 32] = random_bytes().expect("randomness");
    let now = u64::try_from(Utc::now().timestamp_millis()).expect("a positive clock");
    make_bind(&device, &seed, &box_pub, now).encode_to_vec()
}

fn owns(expected: &[u8]) -> impl Fn(&[u8]) -> bool + '_ {
    let keys = CallerBind::decode(expected).expect("bind keys");
    move |stored| keys.owns(stored)
}

async fn oauth_app(pool: &PgPool) -> anyhow::Result<()> {
    let app = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'test', $2, 'secret', 'https://app.test/')",
    )
    .bind(app)
    .bind(app.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

async fn session_for(pool: &PgPool, user: &str, chat_bind: Option<&[u8]>) -> anyhow::Result<Uuid> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, $2, 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .bind(user)
    .execute(pool)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO sessions (id, soundcloud_connection_id, chat_bind) VALUES ($1, $2, $3)",
    )
    .bind(session)
    .bind(connection)
    .bind(chat_bind)
    .execute(pool)
    .await?;
    Ok(session)
}

async fn session_state(pool: &PgPool, session: Uuid) -> anyhow::Result<(String, Option<Vec<u8>>)> {
    Ok(sqlx::query_as(
        "SELECT connection.soundcloud_user_id, session.chat_bind
         FROM sessions AS session
         JOIN soundcloud_connections AS connection ON connection.id = session.soundcloud_connection_id
         WHERE session.id = $1",
    )
    .bind(session)
    .fetch_one(pool)
    .await?)
}

async fn finish(
    pool: &PgPool,
    chat: &ChatState,
    login_request_id: Uuid,
    user: &str,
) -> anyhow::Result<Uuid> {
    let state: String = sqlx::query_scalar("SELECT state FROM login_requests WHERE id = $1")
        .bind(login_request_id)
        .fetch_one(pool)
        .await?;
    let claimed = sqlx::query_file_as!(
        LoginRequest,
        "queries/auth/service/claim_login_request.sql",
        state
    )
    .fetch_one(pool)
    .await?;
    let token = ScTokenResponse {
        access_token: format!("access-{user}"),
        refresh_token: format!("refresh-{user}"),
        expires_in: 3_600,
        scope: String::new(),
        token_type: "bearer".to_owned(),
    };
    let oauth_app_id: Uuid = claimed
        .oauth_app_id
        .as_deref()
        .unwrap_or_default()
        .parse()?;
    Ok(chat
        .auth
        .finish_login(
            &claimed,
            LoginIdentity {
                soundcloud_user_id: user,
                username: Some(user),
                profile: None,
                oauth_app_id,
                token: &token,
                expires_at: Utc::now() + chrono::Duration::hours(1),
                observation: catalog_ingest::Observation::begin(pool).await?,
            },
        )
        .await?)
}

#[sqlx::test(migrations = "./migrations")]
async fn a_sealed_login_copies_its_bind_into_the_new_session(pool: PgPool) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let chat = state(&pool)?;
    let device = bind();
    let login = chat.auth.initiate_bound_login(None, &device).await?;

    let session = finish(&pool, &chat, login.login_request_id, "5101").await?;

    assert_eq!(
        session_state(&pool, session).await?,
        ("5101".to_owned(), Some(device.clone()))
    );
    let mine = chat
        .auth
        .login_request_status_for(login.login_request_id, owns(&device))
        .await?;
    assert_eq!(
        (mine.status.as_str(), mine.session_id),
        ("completed", Some(session))
    );
    let theirs = chat
        .auth
        .login_request_status_for(login.login_request_id, owns(&bind()))
        .await?;
    assert_eq!(
        (theirs.status.as_str(), theirs.session_id),
        ("completed", None)
    );
    let plain = chat
        .auth
        .get_login_request_status(login.login_request_id)
        .await?;
    assert_eq!(plain.session_id, None);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_relogin_into_the_same_user_keeps_the_session_and_takes_the_new_bind(
    pool: PgPool,
) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let chat = state(&pool)?;
    let old = bind();
    let new = bind();
    let session = session_for(&pool, "5102", Some(&old)).await?;
    let login = chat.auth.initiate_bound_login(Some(session), &new).await?;

    let attached = finish(&pool, &chat, login.login_request_id, "5102").await?;

    assert_eq!(attached, session);
    assert_eq!(
        session_state(&pool, session).await?,
        ("5102".to_owned(), Some(new))
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_relogin_into_another_user_gets_a_new_session_and_leaves_the_old_one_alone(
    pool: PgPool,
) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let chat = state(&pool)?;
    let victim = bind();
    let intruder = bind();
    let session = session_for(&pool, "5103", Some(&victim)).await?;
    let login = chat
        .auth
        .initiate_bound_login(Some(session), &intruder)
        .await?;

    let created = finish(&pool, &chat, login.login_request_id, "5199").await?;

    assert_ne!(created, session);
    assert_eq!(
        session_state(&pool, created).await?,
        ("5199".to_owned(), Some(intruder))
    );
    assert_eq!(
        session_state(&pool, session).await?,
        ("5103".to_owned(), Some(victim))
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_plain_relogin_into_another_user_cannot_take_over_a_bound_session(
    pool: PgPool,
) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let chat = state(&pool)?;
    let victim = bind();
    let session = session_for(&pool, "5104", Some(&victim)).await?;
    let login = chat.auth.initiate_login(Some(session)).await?;

    let created = finish(&pool, &chat, login.login_request_id, "5198").await?;

    assert_ne!(created, session);
    assert_eq!(
        session_state(&pool, created).await?,
        ("5198".to_owned(), None)
    );
    assert_eq!(
        session_state(&pool, session).await?,
        ("5104".to_owned(), Some(victim))
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_plain_login_still_works_unchanged(pool: PgPool) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let chat = state(&pool)?;
    let fresh = chat.auth.initiate_login(None).await?;
    let session = finish(&pool, &chat, fresh.login_request_id, "5105").await?;
    assert_eq!(
        session_state(&pool, session).await?,
        ("5105".to_owned(), None)
    );
    let status = chat
        .auth
        .get_login_request_status(fresh.login_request_id)
        .await?;
    assert_eq!(
        (status.status.as_str(), status.session_id),
        ("completed", Some(session))
    );

    let device = bind();
    sqlx::query("UPDATE sessions SET chat_bind = $2 WHERE id = $1")
        .bind(session)
        .bind(&device)
        .execute(&pool)
        .await?;
    let again = chat.auth.initiate_login(Some(session)).await?;
    let attached = finish(&pool, &chat, again.login_request_id, "5105").await?;
    assert_eq!(attached, session);
    assert_eq!(
        session_state(&pool, session).await?,
        ("5105".to_owned(), Some(device))
    );
    Ok(())
}

async fn bound_of(pool: &PgPool, session: Uuid) -> anyhow::Result<Option<Vec<u8>>> {
    Ok(session_state(pool, session).await?.1)
}

#[sqlx::test(migrations = "./migrations")]
async fn a_pull_link_claimed_by_the_bound_source_gives_the_new_session_the_asking_bind(
    pool: PgPool,
) -> anyhow::Result<()> {
    let chat = state(&pool)?;
    let source_bind = bind();
    let source = session_for(&pool, "5106", Some(&source_bind)).await?;
    let fresh = bind();
    let created = chat
        .link
        .create_bound("pull", None, Some(&fresh), |_| false)
        .await?;

    let claimed = chat
        .link
        .claim_bound(
            &created.claim_token,
            Some(source),
            Some(&source_bind),
            owns(&source_bind),
        )
        .await?;

    assert_eq!(
        session_state(&pool, claimed.session_id).await?,
        ("5106".to_owned(), Some(fresh.clone()))
    );
    let mine = chat
        .link
        .get_status_for(created.link_request_id, owns(&fresh))
        .await?;
    assert_eq!(mine.session_id, Some(claimed.session_id));
    let plain = chat.link.get_status(created.link_request_id).await?;
    assert_eq!((plain.status.as_str(), plain.session_id), ("claimed", None));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_pull_link_claimed_by_a_session_id_alone_leaves_the_new_session_unbound(
    pool: PgPool,
) -> anyhow::Result<()> {
    let chat = state(&pool)?;
    let source_bind = bind();
    let source = session_for(&pool, "5109", Some(&source_bind)).await?;
    let thief = bind();
    let fresh = bind();

    let plain = chat
        .link
        .create_bound("pull", None, Some(&fresh), |_| false)
        .await?;
    let claimed = chat.link.claim(&plain.claim_token, Some(source)).await?;
    assert_eq!(bound_of(&pool, claimed.session_id).await?, None);
    let mine = chat
        .link
        .get_status_for(plain.link_request_id, owns(&fresh))
        .await?;
    assert_eq!(mine.session_id, Some(claimed.session_id));
    let theirs = chat.link.get_status(plain.link_request_id).await?;
    assert_eq!(theirs.session_id, None);

    let forged = chat
        .link
        .create_bound("pull", None, Some(&fresh), |_| false)
        .await?;
    let claimed = chat
        .link
        .claim_bound(
            &forged.claim_token,
            Some(source),
            Some(&thief),
            owns(&thief),
        )
        .await?;
    assert_eq!(bound_of(&pool, claimed.session_id).await?, None);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_push_link_from_the_bound_source_gives_the_new_session_the_claiming_bind(
    pool: PgPool,
) -> anyhow::Result<()> {
    let chat = state(&pool)?;
    let source_bind = bind();
    let source = session_for(&pool, "5107", Some(&source_bind)).await?;
    let created = chat
        .link
        .create_bound("push", Some(source), Some(&source_bind), owns(&source_bind))
        .await?;
    let fresh = bind();

    let claimed = chat
        .link
        .claim_bound(&created.claim_token, None, Some(&fresh), owns(&fresh))
        .await?;

    assert_eq!(claimed.mode, "push");
    assert_eq!(
        session_state(&pool, claimed.session_id).await?,
        ("5107".to_owned(), Some(fresh))
    );
    let status = chat.link.get_status(created.link_request_id).await?;
    assert_eq!(
        (status.status.as_str(), status.session_id),
        ("claimed", None)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_push_link_from_a_session_id_alone_leaves_the_new_session_unbound(
    pool: PgPool,
) -> anyhow::Result<()> {
    let chat = state(&pool)?;
    let source_bind = bind();
    let source = session_for(&pool, "5108", Some(&source_bind)).await?;
    let thief = bind();

    let plain = chat.link.create("push", Some(source)).await?;
    let claimed = chat
        .link
        .claim_bound(&plain.claim_token, None, Some(&thief), owns(&thief))
        .await?;
    assert_eq!(bound_of(&pool, claimed.session_id).await?, None);

    let forged = chat
        .link
        .create_bound("push", Some(source), Some(&thief), owns(&thief))
        .await?;
    let claimed = chat
        .link
        .claim_bound(&forged.claim_token, None, Some(&thief), owns(&thief))
        .await?;
    assert_eq!(bound_of(&pool, claimed.session_id).await?, None);

    let proven = chat
        .link
        .create_bound("push", Some(source), Some(&source_bind), owns(&source_bind))
        .await?;
    let claimed = chat.link.claim(&proven.claim_token, None).await?;
    assert_eq!(bound_of(&pool, claimed.session_id).await?, None);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_push_link_is_not_bound_once_the_source_was_bound_again(
    pool: PgPool,
) -> anyhow::Result<()> {
    let chat = state(&pool)?;
    let source_bind = bind();
    let source = session_for(&pool, "5110", Some(&source_bind)).await?;
    let created = chat
        .link
        .create_bound("push", Some(source), Some(&source_bind), owns(&source_bind))
        .await?;
    sqlx::query("UPDATE sessions SET chat_bind = $2 WHERE id = $1")
        .bind(source)
        .bind(bind())
        .execute(&pool)
        .await?;
    let fresh = bind();

    let claimed = chat
        .link
        .claim_bound(&created.claim_token, None, Some(&fresh), owns(&fresh))
        .await?;
    assert_eq!(bound_of(&pool, claimed.session_id).await?, None);
    Ok(())
}
