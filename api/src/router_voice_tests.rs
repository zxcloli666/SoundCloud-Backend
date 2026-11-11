use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use backend_contracts::remote::{CommandKind, LinkProvider};

use super::router_search_tests::{
    app, app_with, app_without_star, config, redis_url, remote_hub, remote_prefix, soundcloud_hits,
    state,
};
use crate::bus::nats::NatsService;
use crate::config::{AdmissionCfg, AdmissionLimitCfg, AppConfig};
use crate::modules::remote::confirm::{Confirming, PageTicket, SalutTicket};
use crate::modules::remote::hub::now_ms;
use crate::modules::remote::{Asked, LinkConfirm};

const REDIRECT: &str = "https://social.yandex.net/broker/redirect";
const STATUS_PATH: &str = "/oauth/authorize/status?ticket=";
const SETTLE_BUDGET: Duration = Duration::from_secs(10);
const POLL_EVERY: Duration = Duration::from_millis(50);

async fn session_for(pool: &PgPool, user: &str) -> anyhow::Result<Uuid> {
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
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session)
        .bind(connection)
        .execute(pool)
        .await?;
    Ok(session)
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<(&str, String)>,
) -> anyhow::Result<Reply> {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let body = match body {
        Some((content_type, text)) => {
            request = request.header(header::CONTENT_TYPE, content_type);
            Body::from(text)
        }
        None => Body::empty(),
    };
    answer_to(app, request.body(body)?).await
}

async fn answer_to(app: &axum::Router, request: Request<Body>) -> anyhow::Result<Reply> {
    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1024 * 1024).await?.to_vec();
    Ok(Reply {
        status,
        headers,
        body,
    })
}

fn with_session(session: Uuid) -> Vec<(&'static str, String)> {
    vec![("x-session-id", session.to_string())]
}

async fn json_call(
    app: &axum::Router,
    session: Uuid,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> anyhow::Result<Reply> {
    send(
        app,
        method,
        uri,
        &with_session(session),
        body.map(|body| ("application/json", body.to_string())),
    )
    .await
}

fn form(pairs: &[(&str, &str)]) -> (&'static str, String) {
    (
        "application/x-www-form-urlencoded",
        pairs
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&"),
    )
}

async fn new_code(app: &axum::Router, session: Uuid, provider: &str) -> anyhow::Result<String> {
    let reply = json_call(
        app,
        session,
        "POST",
        "/remote/link-code",
        Some(json!({"provider": provider})),
    )
    .await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    Ok(reply.json()["code"].as_str().unwrap_or_default().to_owned())
}

async fn authorize(app: &axum::Router, code: &str, state: &str) -> anyhow::Result<Reply> {
    let first = send(
        app,
        "POST",
        "/oauth/authorize",
        &[],
        Some(form(&[
            ("client_id", "yandex"),
            ("redirect_uri", REDIRECT),
            ("state", state),
            ("scope", "read"),
            ("code", code),
        ])),
    )
    .await?;
    ticket_flow(app, first).await
}

async fn confirming_desktop(
    app: &axum::Router,
    session: Uuid,
    device: &'static str,
    yes: bool,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let handshake = json_call(
        app,
        session,
        "GET",
        &format!("/remote/wait?device={device}&active=5&since="),
        None,
    )
    .await?
    .json();
    let mut cursor = handshake["cursor"].as_i64().unwrap_or_default();
    let app = app.clone();
    Ok(tokio::spawn(async move {
        loop {
            let waited = json_call(
                &app,
                session,
                "GET",
                &format!("/remote/wait?device={device}&active=5&since={cursor}"),
                None,
            )
            .await
            .map(|reply| reply.json())
            .unwrap_or(Value::Null);
            cursor = waited["cursor"].as_i64().unwrap_or(cursor);
            for command in waited["commands"].as_array().cloned().unwrap_or_default() {
                if command["kind"] == "link_confirm" {
                    let _ = json_call(
                        &app,
                        session,
                        "POST",
                        "/remote/reply",
                        Some(json!({"id": command["id"], "ok": yes, "text": "done", "state": "done"})),
                    )
                    .await;
                }
            }
        }
    }))
}

fn auth_code_of(reply: &Reply) -> String {
    let location = reply
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    url::Url::parse(location)
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "code")
                .map(|(_, value)| value.into_owned())
        })
        .unwrap_or_default()
}

async fn token_call(app: &axum::Router, pairs: &[(&str, &str)]) -> anyhow::Result<Reply> {
    send(app, "POST", "/oauth/token", &[], Some(form(pairs))).await
}

async fn link_alice(app: &axum::Router, session: Uuid) -> anyhow::Result<(String, String)> {
    let confirmer = confirming_desktop(app, session, "confirm-pc", true).await?;
    let code = new_code(app, session, "alice").await?;
    let redirect = authorize(app, &code, "st").await?;
    confirmer.abort();
    let auth_code = auth_code_of(&redirect);
    let tokens = token_call(
        app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &auth_code),
            ("redirect_uri", REDIRECT),
            ("client_id", "yandex"),
            ("client_secret", "s3cret"),
        ],
    )
    .await?;
    assert_eq!(tokens.status, StatusCode::OK, "{}", tokens.text());
    let tokens = tokens.json();
    Ok((
        tokens["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        tokens["refresh_token"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    ))
}

fn alice_body(utterance: &str, new: bool) -> Value {
    json!({
        "meta": {"locale": "ru-RU", "timezone": "Europe/Moscow", "interfaces": {"screen": {}, "account_linking": {}}},
        "session": {
            "message_id": 0,
            "session_id": "2eac4854-fce721f3-b845abba-20d60",
            "skill_id": "skill-1",
            "user": {"user_id": "6C91DA5198D1758C6A9F63A7C5CDDF09359F683B13A18A151FBF4C8B092BB0C2"},
            "application": {"application_id": "47C73714B580ED2469056E71081159529FFC676A4E5B059D684100A3A1C6F1CC"},
            "new": new
        },
        "request": {
            "command": utterance,
            "original_utterance": utterance,
            "nlu": {"tokens": [], "entities": [], "intents": {}},
            "markup": {"dangerous_context": false},
            "type": "SimpleUtterance"
        },
        "version": "1.0"
    })
}

async fn alice_call(app: &axum::Router, token: Option<&str>, body: Value) -> anyhow::Result<Reply> {
    let mut headers = Vec::new();
    if let Some(token) = token {
        headers.push(("authorization", format!("Bearer {token}")));
    }
    send(
        app,
        "POST",
        "/skills/alice",
        &headers,
        Some(("application/json", body.to_string())),
    )
    .await
}

fn salut_body(sub: &str, text: &str, asr: &str, message_id: u64) -> Value {
    json!({
        "sessionId": "86024848-c12b-4056-b58b-93c69b412314",
        "messageId": message_id,
        "uuid": {"userChannel": "B2C", "sub": sub, "userId": "123"},
        "messageName": "MESSAGE_TO_SKILL",
        "payload": {
            "intent": "run_app",
            "projectName": "loli",
            "new_session": false,
            "message": {
                "original_text": text,
                "normalized_text": text,
                "asr_normalized_message": asr,
                "entities": {},
                "tokenized_elements_list": []
            }
        }
    })
}

async fn salut_call(app: &axum::Router, token: Option<&str>, body: Value) -> anyhow::Result<Reply> {
    let mut headers = Vec::new();
    if let Some(token) = token {
        headers.push(("authorization", format!("Bearer {token}")));
    }
    send(
        app,
        "POST",
        "/skills/salut",
        &headers,
        Some(("application/json", body.to_string())),
    )
    .await
}

async fn fake_desktop(
    app: axum::Router,
    session: Uuid,
    device: &'static str,
    answer: &'static str,
) -> anyhow::Result<tokio::task::JoinHandle<Value>> {
    let handshake = json_call(
        &app,
        session,
        "GET",
        &format!("/remote/wait?device={device}&active=5"),
        None,
    )
    .await?
    .json();
    let cursor = handshake["cursor"].as_i64().unwrap_or_default();
    Ok(tokio::spawn(async move {
        let waited = json_call(
            &app,
            session,
            "GET",
            &format!("/remote/wait?device={device}&active=5&since={cursor}"),
            None,
        )
        .await
        .map(|reply| reply.json())
        .unwrap_or(Value::Null);
        if let Some(command) = waited["commands"].get(0) {
            let _ = json_call(
                &app,
                session,
                "POST",
                "/remote/reply",
                Some(json!({
                    "id": command["id"],
                    "ok": true,
                    "speech": answer,
                    "display": answer
                })),
            )
            .await;
        }
        waited
    }))
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn the_link_page_and_the_token_endpoint_follow_the_code_flow(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;

    let page = send(
        &app,
        "GET",
        &format!("/oauth/authorize?response_type=code&client_id=yandex&redirect_uri={}&state=xyz&scope=read", urlencoding::encode(REDIRECT)),
        &[],
        None,
    )
    .await?;
    assert_eq!(page.status, StatusCode::OK);
    let html = page.text();
    assert!(html.contains("шесть цифр"), "{html}");
    assert!(html.contains("prefers-color-scheme: dark"));
    assert!(html.contains("value=\"xyz\""));

    let wrong_target = send(
        &app,
        "GET",
        "/oauth/authorize?response_type=code&client_id=yandex&redirect_uri=https%3A%2F%2Fevil.example%2Fcb&state=xyz",
        &[],
        None,
    )
    .await?;
    assert_eq!(wrong_target.status, StatusCode::BAD_REQUEST);

    let confirmer = confirming_desktop(&app, session, "pc", true).await?;
    let code = new_code(&app, session, "alice").await?;
    assert_eq!(code.len(), 6);

    let wrong = authorize(&app, "000000", "xyz").await?;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    assert!(wrong.text().contains("Код не подошёл"));

    let spaced = format!("{} {}", &code[..3], &code[3..]);
    let granted = authorize(&app, &spaced, "xyz").await?;
    assert_eq!(granted.status, StatusCode::FOUND, "{}", granted.text());
    let location = granted.headers[header::LOCATION].to_str()?.to_owned();
    assert!(location.starts_with(REDIRECT));
    assert!(location.contains("state=xyz") && location.contains("client_id=yandex"));
    let auth_code = auth_code_of(&granted);
    assert!(!auth_code.is_empty());

    confirmer.abort();
    let reused = authorize(&app, &code, "xyz").await?;
    assert_eq!(
        reused.status,
        StatusCode::BAD_REQUEST,
        "a code is single use"
    );

    let refused = token_call(
        &app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &auth_code),
            ("redirect_uri", REDIRECT),
            ("client_id", "yandex"),
            ("client_secret", "nope"),
        ],
    )
    .await?;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);

    let exchange = [
        ("grant_type", "authorization_code"),
        ("code", auth_code.as_str()),
        ("redirect_uri", REDIRECT),
        ("client_id", "yandex"),
        ("client_secret", "s3cret"),
    ];
    let issued = token_call(&app, &exchange).await?;
    assert_eq!(issued.status, StatusCode::OK, "{}", issued.text());
    assert!(
        issued.headers[header::CACHE_CONTROL]
            .to_str()?
            .contains("no-store")
    );
    let tokens = issued.json();
    assert_eq!(tokens["token_type"], "Bearer");
    assert!(tokens["expires_in"].as_i64().unwrap_or_default() >= 3600);
    let stored: String =
        sqlx::query_scalar("SELECT access_hash FROM remote_grants WHERE access_hash IS NOT NULL")
            .fetch_one(&pool)
            .await?;
    assert_ne!(stored, tokens["access_token"].as_str().unwrap_or_default());

    let old_refresh = tokens["refresh_token"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let refreshed = token_call(
        &app,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &old_refresh),
            ("client_id", "yandex"),
            ("client_secret", "s3cret"),
        ],
    )
    .await?;
    assert_eq!(refreshed.status, StatusCode::OK, "{}", refreshed.text());
    let next = refreshed.json();
    assert_ne!(next["refresh_token"], tokens["refresh_token"]);
    assert_ne!(next["access_token"], tokens["access_token"]);

    let retried = token_call(
        &app,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &old_refresh),
            ("client_id", "yandex"),
            ("client_secret", "s3cret"),
        ],
    )
    .await?;
    assert_eq!(retried.status, StatusCode::OK, "{}", retried.text());
    assert_eq!(
        retried.json(),
        next,
        "a retry inside the grace returns the same pair"
    );

    sqlx::query("UPDATE remote_grants SET rotated_at = now() - interval '6 minutes'")
        .execute(&pool)
        .await?;
    let reused = token_call(
        &app,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &old_refresh),
            ("client_id", "yandex"),
            ("client_secret", "s3cret"),
        ],
    )
    .await?;
    assert_eq!(reused.status, StatusCode::BAD_REQUEST);
    let gone: i64 = sqlx::query_scalar("SELECT count(*) FROM remote_grants")
        .fetch_one(&pool)
        .await?;
    assert_eq!(gone, 0, "reuse after the grace revokes the whole grant");
    let current = token_call(
        &app,
        &[
            ("grant_type", "refresh_token"),
            (
                "refresh_token",
                next["refresh_token"].as_str().unwrap_or_default(),
            ),
            ("client_id", "yandex"),
            ("client_secret", "s3cret"),
        ],
    )
    .await?;
    assert_eq!(current.status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_code_redeemed_twice_revokes_the_grant_and_its_tokens(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let confirmer = confirming_desktop(&app, session, "pc", true).await?;
    let code = new_code(&app, session, "alice").await?;
    let auth_code = auth_code_of(&authorize(&app, &code, "st").await?);
    confirmer.abort();
    let exchange = [
        ("grant_type", "authorization_code"),
        ("code", auth_code.as_str()),
        ("redirect_uri", REDIRECT),
        ("client_id", "yandex"),
        ("client_secret", "s3cret"),
    ];
    let issued = token_call(&app, &exchange).await?;
    assert_eq!(issued.status, StatusCode::OK, "{}", issued.text());
    let tokens = issued.json();
    let access = tokens["access_token"].as_str().unwrap_or_default();
    let refresh = tokens["refresh_token"].as_str().unwrap_or_default();
    let known = alice_call(&app, Some(access), alice_body("пауза", true))
        .await?
        .json();
    assert!(
        known["response"]["directives"]["start_account_linking"].is_null(),
        "{known}"
    );

    let replay = token_call(&app, &exchange).await?;
    assert_eq!(replay.status, StatusCode::BAD_REQUEST);
    assert_eq!(replay.json()["error"], "invalid_grant");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM remote_grants")
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0, "the grant the code issued is gone");

    let refreshed = token_call(
        &app,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", "yandex"),
            ("client_secret", "s3cret"),
        ],
    )
    .await?;
    assert_eq!(refreshed.status, StatusCode::BAD_REQUEST);
    assert_eq!(refreshed.json()["error"], "invalid_grant");
    let after = alice_call(&app, Some(access), alice_body("пауза", true))
        .await?
        .json();
    assert!(after["response"]["directives"]["start_account_linking"].is_object());
    let again = token_call(&app, &exchange).await?;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_linked_account_is_listed(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    link_alice(&app, session).await?;
    let listed = json_call(&app, session, "GET", "/remote/links", None)
        .await?
        .json();
    assert_eq!(listed["links"].as_array().map(Vec::len), Some(1));
    assert_eq!(listed["links"][0]["provider"], "alice");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn an_expired_link_code_is_refused(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let code = new_code(&app, session, "alice").await?;
    sqlx::query("UPDATE remote_link_codes SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await?;
    let late = authorize(&app, &code, "s").await?;
    assert_eq!(late.status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_code_for_one_provider_does_not_link_the_other(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let code = new_code(&app, session, "salut").await?;
    let wrong = authorize(&app, &code, "s").await?;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn revoking_a_link_stops_the_skill_from_recognising_the_user(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let (access, _) = link_alice(&app, session).await?;

    let known = alice_call(&app, Some(&access), alice_body("помощь", true))
        .await?
        .json();
    assert!(
        known["response"]["text"]
            .as_str()
            .unwrap_or_default()
            .starts_with("Я Лоля")
    );

    let listed = json_call(&app, session, "GET", "/remote/links", None)
        .await?
        .json();
    let id = listed["links"][0]["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let stranger = session_for(&pool, "202").await?;
    let denied = json_call(
        &app,
        stranger,
        "DELETE",
        &format!("/remote/links/{id}"),
        None,
    )
    .await?;
    assert_eq!(denied.status, StatusCode::NOT_FOUND);
    let revoked = json_call(
        &app,
        session,
        "DELETE",
        &format!("/remote/links/{id}"),
        None,
    )
    .await?;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);

    let after = alice_call(&app, Some(&access), alice_body("пауза", true))
        .await?
        .json();
    assert!(after["response"]["directives"]["start_account_linking"].is_object());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn alice_answers_ping_help_linking_offline_and_a_real_command(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;

    let ping = alice_call(&app, None, alice_body("ping", false)).await?;
    assert_eq!(ping.json()["response"]["text"], "pong");

    let mut stranger = alice_body("привет", true);
    stranger["session"]["skill_id"] = json!("someone-elses-skill");
    assert_eq!(
        alice_call(&app, None, stranger).await?.status,
        StatusCode::FORBIDDEN
    );

    let unlinked = alice_call(&app, None, alice_body("включи следующий трек", true))
        .await?
        .json();
    assert!(unlinked["response"]["directives"]["start_account_linking"].is_object());
    assert_eq!(unlinked["version"], "1.0");

    let (access, _) = link_alice(&app, session).await?;

    let help = alice_call(&app, Some(&access), alice_body("что ты умеешь", false))
        .await?
        .json();
    assert!(
        help["response"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("Управляю музыкой")
    );
    assert_eq!(help["response"]["end_session"], false);

    let offline = alice_call(&app, Some(&access), alice_body("следующий трек", true))
        .await?
        .json();
    assert_eq!(
        offline["response"]["text"],
        "Компьютер не отвечает. Откройте SoundCloud Desktop."
    );

    let desktop = fake_desktop(app.clone(), session, "pc", "Включаю следующий трек").await?;
    let answered = alice_call(&app, Some(&access), alice_body("следующий трек", true))
        .await?
        .json();
    assert_eq!(answered["response"]["text"], "Включаю следующий трек");
    assert_eq!(answered["response"]["tts"], "Включаю следующий трек");
    assert_eq!(answered["response"]["end_session"], true);
    let delivered = desktop.await?;
    assert_eq!(delivered["commands"][0]["text"], "следующий трек");
    assert_eq!(delivered["commands"][0]["source"], "alice");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn salut_links_by_voice_and_then_relays_commands(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "303").await?;
    let sub = "d2d6da62-6bdd-452b-b5dd-a145090075ba";

    assert_eq!(
        salut_call(&app, None, salut_body(sub, "привет", "привет", 1))
            .await?
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        salut_call(&app, Some("wrong"), salut_body(sub, "привет", "привет", 1))
            .await?
            .status,
        StatusCode::UNAUTHORIZED
    );

    let prompt = salut_call(
        &app,
        Some("salut-token"),
        salut_body(sub, "включи музыку", "Включи музыку", 7),
    )
    .await?;
    assert_eq!(prompt.status, StatusCode::OK);
    let prompt = prompt.json();
    assert_eq!(prompt["messageName"], "ANSWER_TO_USER");
    assert_eq!(prompt["messageId"], 7);
    assert_eq!(prompt["uuid"]["sub"], sub);
    assert!(
        prompt["payload"]["pronounceText"]
            .as_str()
            .unwrap_or_default()
            .contains("шесть цифр")
    );

    let code = new_code(&app, session, "salut").await?;
    let wrong = salut_call(
        &app,
        Some("salut-token"),
        salut_body(sub, "ноль ноль ноль", "0 0 0", 8),
    )
    .await?
    .json();
    assert!(
        wrong["payload"]["pronounceText"]
            .as_str()
            .unwrap_or_default()
            .contains("шесть")
    );

    let spoken = code
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let confirmer = confirming_desktop(&app, session, "pc", true).await?;
    let asked = salut_call(
        &app,
        Some("salut-token"),
        salut_body(sub, &spoken, &spoken, 9),
    )
    .await?
    .json();
    assert_eq!(asked["messageId"], 9);
    assert!(
        asked["payload"]["pronounceText"]
            .as_str()
            .unwrap_or_default()
            .contains("Подтвердите на компьютере")
    );
    let linked = salut_until(&app, sub, "ну что", 20, |said| {
        said.contains("аккаунт связан")
    })
    .await?;
    confirmer.abort();
    assert!(linked.contains("аккаунт связан"), "{linked}");

    let offline = salut_call(
        &app,
        Some("salut-token"),
        salut_body(sub, "пауза", "Пауза", 10),
    )
    .await?
    .json();
    assert!(
        offline["payload"]["pronounceText"]
            .as_str()
            .unwrap_or_default()
            .contains("Компьютер не отвечает")
    );

    let desktop = fake_desktop(app.clone(), session, "pc", "Ставлю на паузу").await?;
    let answered = salut_call(
        &app,
        Some("salut-token"),
        salut_body(sub, "пауза", "Пауза", 11),
    )
    .await?
    .json();
    assert_eq!(answered["payload"]["pronounceText"], "Ставлю на паузу");
    assert_eq!(
        answered["payload"]["items"][0]["bubble"]["text"],
        "Ставлю на паузу"
    );
    assert_eq!(answered["messageId"], 11);
    assert_eq!(desktop.await?["commands"][0]["source"], "salut");

    let listed = json_call(&app, session, "GET", "/remote/links", None)
        .await?
        .json();
    assert_eq!(listed["links"][0]["provider"], "salut");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn the_remote_endpoints_need_a_session(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    for (method, uri) in [
        ("GET", "/remote/wait"),
        ("POST", "/remote/reply"),
        ("POST", "/remote/link-code"),
        ("GET", "/remote/links"),
    ] {
        let reply = send(&app, method, uri, &[], None).await?;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn resolve_merges_variants_and_matches_case_forms(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, track_count_primary, popularity_score)
         VALUES ('00000000-0000-0000-0000-0000000000a1', 'Земфира', 'земфира', 'test', 40, 0.9),
                ('00000000-0000-0000-0000-0000000000a2', 'Земфирка Мэш', 'земфирка мэш', 'test', 3, 0.1)",
    )
    .execute(&pool)
    .await?;
    crate::db::insert_replicated(
        &pool,
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, metadata_artist,
                             uploader_username, uploader_sc_user_id, duration_ms, play_count_sc, sharing)
         VALUES ('9001', 'soundcloud:tracks:9001', 'Хочешь', 'хочешь', 'Земфира', 'Земфира', '555', 200000, 50000, 'public'),
                ('9002', 'soundcloud:tracks:9002', 'Хочешь (remix)', 'хочешь remix', 'Земфира', 'Земфира', '555', 210000, 10, 'public')",
    )
    .await?;
    crate::modules::search::lexicon_refresh::refresh(&pool).await?;
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;

    let found = json_call(
        &app,
        session,
        "POST",
        "/search/resolve",
        Some(json!({
            "variants": ["песни земфиры", "земфиры", "земфира хочешь", "земфиры"],
            "kinds": ["artist", "track"],
            "limit": 3
        })),
    )
    .await?;
    assert_eq!(found.status, StatusCode::OK, "{}", found.text());
    let body = found.json();
    let artists = body["results"]["artist"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(artists[0]["name"], "Земфира", "{body}");
    assert_eq!(artists[0]["kind"], "artist");
    assert!(artists[0]["score"].as_f64().unwrap_or_default() > 0.3);
    assert!(artists[0]["popularity"].as_f64().unwrap_or_default() > 0.5);
    let ids: Vec<&str> = artists.iter().filter_map(|a| a["id"].as_str()).collect();
    let unique: std::collections::HashSet<&&str> = ids.iter().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "variants merge into one row per entity"
    );

    let tracks = body["results"]["track"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(tracks[0]["id"], "soundcloud:tracks:9001", "{body}");
    assert_eq!(tracks[0]["artist"], "Земфира");
    assert!(body["results"].get("album").is_none());

    let bad = json_call(
        &app,
        session,
        "POST",
        "/search/resolve",
        Some(json!({"variants": ["x"], "kinds": ["podcast"]})),
    )
    .await?;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);

    let empty = json_call(
        &app,
        session,
        "POST",
        "/search/resolve",
        Some(json!({"variants": []})),
    )
    .await?;
    assert_eq!(empty.status, StatusCode::OK);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn the_link_page_refuses_framing_and_waits_for_the_desktop(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id=yandex&redirect_uri={}&state=s",
        urlencoding::encode(REDIRECT)
    );
    let page = send(&app, "GET", &uri, &[], None).await?;
    assert_eq!(page.headers["x-frame-options"], "DENY");
    assert!(
        page.headers["content-security-policy"]
            .to_str()?
            .contains("frame-ancestors 'none'")
    );

    let _confirmer = confirming_desktop(&app, session, "pc", true).await?;
    let code = new_code(&app, session, "alice").await?;
    let posted = send(
        &app,
        "POST",
        "/oauth/authorize",
        &[],
        Some(form(&[
            ("client_id", "yandex"),
            ("redirect_uri", REDIRECT),
            ("state", "s"),
            ("code", &code),
        ])),
    )
    .await?;
    assert_eq!(posted.status, StatusCode::OK);
    assert!(posted.text().contains("Подтвердите на компьютере"));
    assert!(posted.text().contains("http-equiv=\"refresh\""));
    assert_eq!(posted.headers["x-frame-options"], "DENY");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_no_from_the_desktop_issues_nothing_and_burns_the_code(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let confirmer = confirming_desktop(&app, session, "pc", false).await?;
    let code = new_code(&app, session, "alice").await?;
    let denied = authorize(&app, &code, "s").await?;
    assert_eq!(denied.status, StatusCode::OK);
    assert!(denied.text().contains("отклонено"), "{}", denied.text());
    let codes: i64 = sqlx::query_scalar("SELECT count(*) FROM remote_grants")
        .fetch_one(&pool)
        .await?;
    assert_eq!(codes, 0, "no auth code without a yes");
    let again = authorize(&app, &code, "s").await?;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    confirmer.abort();
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn without_a_desktop_the_link_is_not_made_and_the_code_survives(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let code = new_code(&app, session, "alice").await?;
    let offline = authorize(&app, &code, "s").await?;
    assert!(offline.text().contains("не отвечает"), "{}", offline.text());
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM remote_grants")
        .fetch_one(&pool)
        .await?;
    assert_eq!(grants, 0);
    let confirmer = confirming_desktop(&app, session, "pc", true).await?;
    let retry = authorize(&app, &code, "s").await?;
    assert_eq!(retry.status, StatusCode::FOUND, "{}", retry.text());
    confirmer.abort();
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn salut_binds_only_after_a_yes_and_a_no_binds_nothing(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "303").await?;
    let sub = "11111111-2222-3333-4444-555555555555";
    let confirmer = confirming_desktop(&app, session, "pc", false).await?;
    let code = new_code(&app, session, "salut").await?;
    let spoken = code
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let first = salut_call(
        &app,
        Some("salut-token"),
        salut_body(sub, &spoken, &spoken, 1),
    )
    .await?
    .json();
    assert!(
        first["payload"]["pronounceText"]
            .as_str()
            .unwrap_or_default()
            .contains("Подтвердите")
    );
    let text = salut_until(&app, sub, "а", 2, |said| !said.contains("Подтвердите")).await?;
    confirmer.abort();
    assert!(text.contains("отклонена"), "{text}");
    let bound: i64 =
        sqlx::query_scalar("SELECT count(*) FROM remote_grants WHERE external_id IS NOT NULL")
            .fetch_one(&pool)
            .await?;
    assert_eq!(bound, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_code_is_asked_at_most_five_times(pool: PgPool) -> anyhow::Result<()> {
    use crate::modules::remote::linking::{self, Found};
    let st = state(&pool, soundcloud_hits(), &redis_url()).await?;
    let issued = st.links.create_code("101", LinkProvider::Alice).await?;
    st.remote
        .wait("101", "pc", 1, None, Duration::ZERO)
        .await
        .map_err(|refused| anyhow::anyhow!("{refused:?}"))?;
    for _ in 0..5 {
        let Found::Ready(pairing) = linking::find(&st, LinkProvider::Alice, &issued.code).await?
        else {
            anyhow::bail!("the code is still usable before the sixth try");
        };
        assert!(linking::ask(&st, &pairing).await.is_ok());
        st.confirm.release(pairing.code_id, &pairing.holder).await;
    }
    assert!(matches!(
        linking::find(&st, LinkProvider::Alice, &issued.code).await?,
        Found::NoCode
    ));
    assert!(
        st.links
            .find_code(LinkProvider::Alice, &issued.code)
            .await?
            .is_none(),
        "the sixth try burns the code"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn the_desktop_shapes_are_tolerated(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let first = json_call(
        &app,
        session,
        "GET",
        "/remote/wait?since=&device=pc&active=",
        None,
    )
    .await?;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert!(
        first.json()["cursor"]
            .as_i64()
            .is_some_and(|cursor| cursor >= 0)
    );
    let bad = json_call(&app, session, "GET", "/remote/wait?since=abc", None).await?;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);

    let (access, _) = link_alice(&app, session).await?;
    let fresh = json_call(
        &app,
        session,
        "GET",
        "/remote/wait?device=pc&active=100",
        None,
    )
    .await?
    .json();
    let desktop = tokio::spawn({
        let app = app.clone();
        async move {
            let waited = json_call(
                &app,
                session,
                "GET",
                &format!(
                    "/remote/wait?device=pc&active=100&since={}",
                    fresh["cursor"]
                ),
                None,
            )
            .await
            .map(|reply| reply.json())
            .unwrap_or(Value::Null);
            if let Some(command) = waited["commands"].get(0) {
                let _ = json_call(
                    &app,
                    session,
                    "POST",
                    "/remote/reply",
                    Some(json!({"id": command["id"], "ok": true, "text": "Из поля text", "state": "done"})),
                )
                .await;
            }
        }
    });
    let answered = alice_call(&app, Some(&access), alice_body("пауза", true))
        .await?
        .json();
    desktop.await?;
    assert_eq!(answered["response"]["text"], "Из поля text");
    let links = json_call(&app, session, "GET", "/remote/links", None)
        .await?
        .json();
    assert!(links["links"].is_array());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn waits_per_user_are_capped_and_a_repeat_replaces_the_first(
    pool: PgPool,
) -> anyhow::Result<()> {
    let st = state(&pool, soundcloud_hits(), &redis_url()).await?;
    let remote = st.remote.clone();
    let app = crate::router::build(st);
    let session = session_for(&pool, "101").await?;
    let cursor = json_call(&app, session, "GET", "/remote/wait?device=a", None)
        .await?
        .json()["cursor"]
        .as_i64()
        .unwrap_or_default();
    let wait = |device: &'static str| {
        let app = app.clone();
        tokio::spawn(async move {
            json_call(
                &app,
                session,
                "GET",
                &format!("/remote/wait?device={device}&active=1&since={cursor}"),
                None,
            )
            .await
        })
    };
    let first_a = wait("a");
    let _b = wait("b");
    let _c = wait("c");
    let deadline = tokio::time::Instant::now() + SETTLE_BUDGET;
    while remote.held("101") < 3 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(POLL_EVERY).await;
    }
    let fourth = json_call(
        &app,
        session,
        "GET",
        &format!("/remote/wait?device=d&active=1&since={cursor}"),
        None,
    )
    .await?;
    assert_eq!(fourth.status, StatusCode::TOO_MANY_REQUESTS);
    let second_a = wait("a");
    let replaced = tokio::time::timeout(Duration::from_secs(2), first_a).await??;
    assert_eq!(replaced?.status, StatusCode::OK);
    assert!(!second_a.is_finished());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn reply_is_capped_and_needs_star(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let big = json!({"id": Uuid::now_v7(), "ok": true, "state": "x".repeat(20_000)});
    let refused = json_call(&app, session, "POST", "/remote/reply", Some(big)).await?;
    assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE);
    let small = json!({"id": Uuid::now_v7(), "ok": true});
    let unknown = json_call(&app, session, "POST", "/remote/reply", Some(small.clone())).await?;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    assert_eq!(unknown.json()["code"], "voice_reply_unknown");

    let free = app_without_star(&pool, &redis_url()).await?;
    let no_star = json_call(&free, session, "POST", "/remote/reply", Some(small)).await?;
    assert_eq!(no_star.status, StatusCode::FORBIDDEN);
    let resolve = json_call(
        &free,
        session,
        "POST",
        "/search/resolve",
        Some(json!({"variants": ["x"]})),
    )
    .await?;
    assert_eq!(resolve.status, StatusCode::FORBIDDEN);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn resolve_with_a_hostile_body_stays_fast(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let started = std::time::Instant::now();
    let reply = json_call(
        &app,
        session,
        "POST",
        "/search/resolve",
        Some(json!({
            "variants": (0..150).map(|n| format!("{}{n}", "я".repeat(300))).collect::<Vec<_>>(),
            "kinds": vec!["track"; 5_000],
        })),
    )
    .await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn alice_with_a_silent_desktop_stays_fast(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let (access, _) = link_alice(&app, session).await?;
    json_call(
        &app,
        session,
        "GET",
        "/remote/wait?device=quiet&active=9",
        None,
    )
    .await?;
    let started = std::time::Instant::now();
    let reply = alice_call(&app, Some(&access), alice_body("пауза", true))
        .await?
        .json();
    let took = started.elapsed();
    assert!(
        reply["response"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("не отвечает")
    );
    assert!(
        took >= Duration::from_millis(400) && took < Duration::from_millis(2_500),
        "the skill waits for the desktop and still answers inside the Alice budget: {took:?}"
    );
    Ok(())
}

fn tuned(tweak: impl FnOnce(&mut AppConfig)) -> AppConfig {
    let mut config = config(&redis_url());
    tweak(&mut config);
    config
}

fn cap(per_client: u32) -> AdmissionLimitCfg {
    AdmissionLimitCfg {
        per_client,
        global: 1_000_000,
    }
}

fn said(reply: &Reply) -> String {
    let body = reply.json();
    body["response"]["text"]
        .as_str()
        .or_else(|| body["payload"]["pronounceText"].as_str())
        .unwrap_or_default()
        .to_owned()
}

fn hint_on(page: &str) -> String {
    let marker = "class=\"hint\" aria-label=\"Число для проверки\">";
    page.find(marker)
        .map(|at| {
            page[at + marker.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect()
        })
        .unwrap_or_default()
}

fn ticket_in(page: &str) -> Option<String> {
    let at = page.find(STATUS_PATH)?;
    Some(
        page[at + STATUS_PATH.len()..]
            .chars()
            .take_while(|c| *c != '"')
            .collect(),
    )
}

async fn ticket_flow(app: &axum::Router, first: Reply) -> anyhow::Result<Reply> {
    let deadline = tokio::time::Instant::now() + SETTLE_BUDGET;
    let mut reply = first;
    loop {
        let Some(ticket) = ticket_in(&reply.text()) else {
            return Ok(reply);
        };
        if tokio::time::Instant::now() >= deadline {
            return Ok(reply);
        }
        tokio::time::sleep(POLL_EVERY).await;
        reply = send(app, "GET", &format!("{STATUS_PATH}{ticket}"), &[], None).await?;
    }
}

async fn salut_until(
    app: &axum::Router,
    sub: &str,
    text: &str,
    first_id: u64,
    done: impl Fn(&str) -> bool,
) -> anyhow::Result<String> {
    let deadline = tokio::time::Instant::now() + SETTLE_BUDGET;
    let mut message_id = first_id;
    loop {
        let reply = salut_call(
            app,
            Some("salut-token"),
            salut_body(sub, text, text, message_id),
        )
        .await?
        .json();
        let said = reply["payload"]["pronounceText"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if done(&said) || tokio::time::Instant::now() >= deadline {
            return Ok(said);
        }
        message_id += 1;
        tokio::time::sleep(POLL_EVERY).await;
    }
}

async fn submit_code(app: &axum::Router, code: &str) -> anyhow::Result<Reply> {
    send(
        app,
        "POST",
        "/oauth/authorize",
        &[],
        Some(form(&[
            ("client_id", "yandex"),
            ("redirect_uri", REDIRECT),
            ("state", "s"),
            ("code", code),
        ])),
    )
    .await
}

async fn exchange(app: &axum::Router, auth_code: &str) -> anyhow::Result<String> {
    let tokens = token_call(
        app,
        &[
            ("grant_type", "authorization_code"),
            ("code", auth_code),
            ("redirect_uri", REDIRECT),
            ("client_id", "yandex"),
            ("client_secret", "s3cret"),
        ],
    )
    .await?;
    assert_eq!(tokens.status, StatusCode::OK, "{}", tokens.text());
    Ok(tokens.json()["access_token"]
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

fn salut_for_project(sub: &str, text: &str, project: &str) -> Value {
    let mut body = salut_body(sub, text, text, 1);
    body["payload"]["app_info"] = json!({"projectId": project, "applicationId": "a"});
    body
}

async fn salut_at(app: &axum::Router, uri: &str, body: Value) -> anyhow::Result<Reply> {
    send(
        app,
        "POST",
        uri,
        &[],
        Some(("application/json", body.to_string())),
    )
    .await
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn every_wait_is_rate_limited_per_session(pool: PgPool) -> anyhow::Result<()> {
    let app = app_with(
        &pool,
        tuned(|config| config.admission.voice_remote_wait = cap(3)),
        true,
    )
    .await?;
    let session = session_for(&pool, "101").await?;
    let mut cursor = 0;
    for _ in 0..3 {
        let reply = json_call(
            &app,
            session,
            "GET",
            "/remote/wait?device=pc&active=1",
            None,
        )
        .await?;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        cursor = reply.json()["cursor"].as_i64().unwrap_or_default();
    }
    let handshake = json_call(
        &app,
        session,
        "GET",
        "/remote/wait?device=pc&active=1",
        None,
    )
    .await?;
    assert_eq!(handshake.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(handshake.json()["code"], "voice_remote_limited");
    assert!(handshake.headers.contains_key(header::RETRY_AFTER));
    let resumed = json_call(
        &app,
        session,
        "GET",
        &format!("/remote/wait?device=pc&active=1&since={cursor}"),
        None,
    )
    .await?;
    assert_eq!(
        resumed.status,
        StatusCode::TOO_MANY_REQUESTS,
        "a resume wait counts too"
    );
    let other = session_for(&pool, "202").await?;
    let fresh = json_call(&app, other, "GET", "/remote/wait?device=pc", None).await?;
    assert_eq!(fresh.status, StatusCode::OK);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn the_link_hint_on_the_page_matches_the_one_sent_to_the_desktop(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let handshake = json_call(
        &app,
        session,
        "GET",
        "/remote/wait?device=pc&active=5",
        None,
    )
    .await?
    .json();
    let cursor = handshake["cursor"].as_i64().unwrap_or_default();
    let desktop = tokio::spawn({
        let app = app.clone();
        async move {
            let waited = json_call(
                &app,
                session,
                "GET",
                &format!("/remote/wait?device=pc&active=5&since={cursor}"),
                None,
            )
            .await
            .map(|reply| reply.json())
            .unwrap_or(Value::Null);
            let command = waited["commands"][0].clone();
            let _ = json_call(
                &app,
                session,
                "POST",
                "/remote/reply",
                Some(json!({"id": command["id"], "ok": true})),
            )
            .await;
            command
        }
    });
    let code = new_code(&app, session, "alice").await?;
    let pending = submit_code(&app, &code).await?;
    assert_eq!(pending.status, StatusCode::OK, "{}", pending.text());
    let hint = hint_on(&pending.text());
    assert_eq!(hint.len(), 2, "{}", pending.text());
    let command = desktop.await?;
    assert_eq!(command["kind"], "link_confirm");
    assert_eq!(command["hint"], hint.as_str());
    let done = ticket_flow(&app, pending).await?;
    assert_eq!(done.status, StatusCode::FOUND, "{}", done.text());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn salut_says_the_hint_after_the_code(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "303").await?;
    let _desktop = json_call(
        &app,
        session,
        "GET",
        "/remote/wait?device=pc&active=5",
        None,
    )
    .await?;
    let code = new_code(&app, session, "salut").await?;
    let spoken = code
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let sub = "44444444-2222-3333-4444-555555555555";
    let asked = salut_call(
        &app,
        Some("salut-token"),
        salut_body(sub, &spoken, &spoken, 1),
    )
    .await?;
    let text = said(&asked);
    assert!(
        text.starts_with("Подтвердите на компьютере. Там будет число "),
        "{text}"
    );
    let again = salut_call(&app, Some("salut-token"), salut_body(sub, "ну", "ну", 2)).await?;
    assert_eq!(
        said(&again),
        text,
        "the pending answer repeats the same hint"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_code_written_a_moment_later_is_still_found(pool: PgPool) -> anyhow::Result<()> {
    use crate::modules::remote::linking::find_code_with_retry;
    let links = crate::modules::remote::LinksService::new(pool.clone());
    let late = |code: &'static str| {
        let pool = pool.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            sqlx::query(
                "INSERT INTO remote_link_codes (id, sc_user_id, provider, code_hash, expires_at)
                 VALUES ($1, '101', 'alice', $2, now() + interval '10 minutes')",
            )
            .bind(Uuid::now_v7())
            .bind(crate::modules::remote::secrets::code_hash(
                LinkProvider::Alice,
                code,
            ))
            .execute(&pool)
            .await
        })
    };
    let writer = late("424242");
    let found = find_code_with_retry(
        &links,
        LinkProvider::Alice,
        "424242",
        Duration::from_millis(500),
    )
    .await?;
    writer.await??;
    assert_eq!(found.map(|code| code.user).as_deref(), Some("101"));

    let writer = late("515151");
    let missed = find_code_with_retry(
        &links,
        LinkProvider::Alice,
        "515151",
        Duration::from_millis(10),
    )
    .await?;
    writer.await??;
    assert!(
        missed.is_none(),
        "without the wait the late row is not seen"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn alice_with_a_stuck_database_stays_fast(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let (access, _) = link_alice(&app, session).await?;
    let mut held = Vec::new();
    for _ in 0..pool.options().get_max_connections() {
        held.push(pool.acquire().await?);
    }
    let started = std::time::Instant::now();
    let reply = alice_call(&app, Some(&access), alice_body("пауза", true)).await?;
    let took = started.elapsed();
    drop(held);
    assert_eq!(said(&reply), "Сервис занят. Попробуйте ещё раз.");
    assert!(took < Duration::from_millis(1000), "{took:?}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn hello_and_help_need_no_link_in_either_skill(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let hello = alice_call(&app, None, alice_body("", true)).await?;
    assert!(said(&hello).starts_with("Привет"), "{}", hello.text());
    assert!(hello.json()["response"]["directives"].is_null());
    let help = alice_call(&app, None, alice_body("Что ты умеешь?", false)).await?;
    assert!(said(&help).contains("Управляю музыкой"));
    assert!(help.json()["response"]["directives"].is_null());
    let command = alice_call(&app, None, alice_body("пауза", false)).await?;
    assert!(command.json()["response"]["directives"]["start_account_linking"].is_object());

    let sub = "55555555-2222-3333-4444-555555555555";
    let help = salut_call(
        &app,
        Some("salut-token"),
        salut_body(sub, "помощь", "помощь", 1),
    )
    .await?;
    assert!(said(&help).contains("Управляю музыкой"));
    let mut start = salut_body(sub, "", "", 2);
    start["messageName"] = json!("RUN_APP");
    start["payload"]["message"] = Value::Null;
    let hello = salut_call(&app, Some("salut-token"), start).await?;
    assert!(said(&hello).starts_with("Привет"), "{}", hello.text());
    let command = salut_call(
        &app,
        Some("salut-token"),
        salut_body(sub, "пауза", "пауза", 3),
    )
    .await?;
    assert!(said(&command).contains("шесть цифр"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn salut_needs_its_token_and_a_project_id_is_only_an_extra_check(
    pool: PgPool,
) -> anyhow::Result<()> {
    let sub = "66666666-2222-3333-4444-555555555555";
    let app = app(&pool, &redis_url()).await?;
    let in_path = salut_at(
        &app,
        "/skills/salut/salut-token",
        salut_body(sub, "помощь", "", 1),
    )
    .await?;
    assert_eq!(in_path.status, StatusCode::OK);
    assert_eq!(in_path.json()["messageName"], "ANSWER_TO_USER");
    let wrong = salut_at(
        &app,
        "/skills/salut/salut-tokeN",
        salut_body(sub, "помощь", "", 1),
    )
    .await?;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);

    let project_only = app_with(
        &pool,
        tuned(|config| {
            config.voice.salut_token = String::new().into();
            config.voice.salut_project_id = "proj".to_owned();
        }),
        true,
    )
    .await?;
    let known = salut_at(
        &project_only,
        "/skills/salut",
        salut_for_project(sub, "пауза", "proj"),
    )
    .await?;
    assert_eq!(known.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(known.json()["code"], "voice_disabled");

    let both = app_with(
        &pool,
        tuned(|config| config.voice.salut_project_id = "proj".to_owned()),
        true,
    )
    .await?;
    let no_token = salut_at(
        &both,
        "/skills/salut",
        salut_for_project(sub, "пауза", "proj"),
    )
    .await?;
    assert_eq!(no_token.status, StatusCode::UNAUTHORIZED);
    let full = salut_at(
        &both,
        "/skills/salut/salut-token",
        salut_for_project(sub, "пауза", "proj"),
    )
    .await?;
    assert_eq!(full.status, StatusCode::OK);
    assert!(said(&full).contains("шесть цифр"));
    let stranger = salut_at(
        &both,
        "/skills/salut/salut-token",
        salut_for_project(sub, "пауза", "other"),
    )
    .await?;
    assert_eq!(stranger.status, StatusCode::FORBIDDEN);

    let off = app_with(
        &pool,
        tuned(|config| config.voice.salut_token = String::new().into()),
        true,
    )
    .await?;
    let disabled = salut_at(
        &off,
        "/skills/salut/anything",
        salut_for_project(sub, "пауза", "proj"),
    )
    .await?;
    assert_eq!(disabled.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(disabled.json()["code"], "voice_disabled");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn the_review_code_links_a_demo_account_that_never_reaches_a_desktop(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let real = session_for(&pool, "101").await?;
    let _real_desktop =
        json_call(&app, real, "GET", "/remote/wait?device=pc&active=5", None).await?;

    let redirected = submit_code(&app, "1234 5678").await?;
    assert_eq!(
        redirected.status,
        StatusCode::FOUND,
        "{}",
        redirected.text()
    );
    let access = exchange(&app, &auth_code_of(&redirected)).await?;
    let owners: Vec<String> = sqlx::query_scalar("SELECT DISTINCT sc_user_id FROM remote_grants")
        .fetch_all(&pool)
        .await?;
    assert_eq!(owners, ["voice-review"]);

    let play = alice_call(&app, Some(&access), alice_body("включи Земфиру", true)).await?;
    assert_eq!(said(&play), "Включаю Земфиру на компьютере.");
    let playing = alice_call(&app, Some(&access), alice_body("что сейчас играет", false)).await?;
    assert!(said(&playing).contains("Night Drive"));
    let quieter = alice_call(&app, Some(&access), alice_body("сделай потише", false)).await?;
    assert_eq!(said(&quieter), "Делаю потише.");

    let sub = "77777777-2222-3333-4444-555555555555";
    let linked = salut_call(
        &app,
        Some("salut-token"),
        salut_body(
            sub,
            "код один два три четыре пять шесть семь восемь",
            "код 1 2 3 4 5 6 7 8",
            1,
        ),
    )
    .await?;
    assert!(
        said(&linked).contains("аккаунт связан"),
        "{}",
        linked.text()
    );
    let paused = salut_call(
        &app,
        Some("salut-token"),
        salut_body(sub, "пауза", "пауза", 2),
    )
    .await?;
    assert_eq!(said(&paused), "Ставлю на паузу.");

    let listed = json_call(&app, real, "GET", "/remote/links", None)
        .await?
        .json();
    assert_eq!(listed["links"].as_array().map(Vec::len), Some(0));

    let off = app_with(
        &pool,
        tuned(|config| config.voice.review_code = String::new().into()),
        true,
    )
    .await?;
    let refused = submit_code(&off, "12345678").await?;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.text().contains("Нужны шесть цифр"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn an_expired_alice_link_is_not_listed(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    link_alice(&app, session).await?;
    sqlx::query("UPDATE remote_grants SET refresh_expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await?;
    let listed = json_call(&app, session, "GET", "/remote/links", None)
        .await?
        .json();
    assert_eq!(listed["links"].as_array().map(Vec::len), Some(0));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn links_open_on_a_premium_host_without_star_but_waits_do_not(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app_with(&pool, tuned(|config| config.premium_reserve = true), false).await?;
    let session = session_for(&pool, "101").await?;
    let links = json_call(&app, session, "GET", "/remote/links", None).await?;
    assert_eq!(links.status, StatusCode::OK, "{}", links.text());
    let wait = json_call(&app, session, "GET", "/remote/wait?device=pc", None).await?;
    assert_eq!(wait.status, StatusCode::FORBIDDEN);
    assert_eq!(wait.json()["code"], "star_required");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn offline_asks_do_not_burn_the_daily_cap(pool: PgPool) -> anyhow::Result<()> {
    let app = app_with(
        &pool,
        tuned(|config| config.voice.confirm_per_user_day = 2),
        true,
    )
    .await?;
    let session = session_for(&pool, "101").await?;
    let code = new_code(&app, session, "alice").await?;
    for _ in 0..4 {
        let offline = authorize(&app, &code, "s").await?;
        assert!(offline.text().contains("не отвечает"), "{}", offline.text());
    }
    let confirmer = confirming_desktop(&app, session, "pc", true).await?;
    let linked = authorize(&app, &code, "s").await?;
    confirmer.abort();
    assert_eq!(linked.status, StatusCode::FOUND, "{}", linked.text());
    Ok(())
}

async fn skill_call_from(
    app: &axum::Router,
    address: std::net::SocketAddr,
    uri: &str,
    token: &str,
    body: Value,
) -> anyhow::Result<Reply> {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body.to_string()))?;
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(address));
    answer_to(app, request).await
}

async fn page_submit_from(
    app: &axum::Router,
    address: std::net::SocketAddr,
    code: &str,
) -> anyhow::Result<Reply> {
    let (content_type, body) = form(&[
        ("client_id", "yandex"),
        ("redirect_uri", REDIRECT),
        ("state", "s"),
        ("code", code),
    ]);
    let mut request = Request::builder()
        .method("POST")
        .uri("/oauth/authorize")
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))?;
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(address));
    answer_to(app, request).await
}

fn random_address() -> std::net::SocketAddr {
    let bytes = Uuid::now_v7().into_bytes();
    let ip = std::net::Ipv6Addr::new(
        0x2001,
        0xdb8,
        u16::from_be_bytes([bytes[14], bytes[15]]),
        u16::from_be_bytes([bytes[12], bytes[13]]),
        0,
        0,
        0,
        1,
    );
    std::net::SocketAddr::new(ip.into(), 443)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn skill_calls_are_limited_per_address_before_any_lookup(pool: PgPool) -> anyhow::Result<()> {
    let app = app_with(
        &pool,
        tuned(|config| {
            config.admission.voice_alice_ip = cap(2);
            config.admission.voice_salut_ip = cap(2);
        }),
        true,
    )
    .await?;
    let yandex = random_address();
    for _ in 0..2 {
        let help = skill_call_from(
            &app,
            yandex,
            "/skills/alice",
            "nope",
            alice_body("помощь", false),
        )
        .await?;
        assert!(said(&help).contains("Управляю музыкой"));
    }
    for _ in 0..2 {
        let unknown = skill_call_from(
            &app,
            yandex,
            "/skills/alice",
            "nope",
            alice_body("пауза", false),
        )
        .await?;
        assert!(unknown.json()["response"]["directives"]["start_account_linking"].is_object());
    }
    let limited = skill_call_from(
        &app,
        yandex,
        "/skills/alice",
        "nope",
        alice_body("пауза", false),
    )
    .await?;
    assert_eq!(said(&limited), "Слишком много команд. Подождите немного.");
    let elsewhere = skill_call_from(
        &app,
        random_address(),
        "/skills/alice",
        "nope",
        alice_body("пауза", false),
    )
    .await?;
    assert!(elsewhere.json()["response"]["directives"]["start_account_linking"].is_object());

    let sber = random_address();
    let sub = "88888888-2222-3333-4444-555555555555";
    for n in 0..2 {
        let asked = skill_call_from(
            &app,
            sber,
            "/skills/salut",
            "salut-token",
            salut_body(sub, "пауза", "пауза", n),
        )
        .await?;
        assert!(said(&asked).contains("шесть цифр"), "{}", asked.text());
    }
    let limited = skill_call_from(
        &app,
        sber,
        "/skills/salut",
        "salut-token",
        salut_body(sub, "пауза", "пауза", 9),
    )
    .await?;
    assert_eq!(said(&limited), "Слишком много команд. Подождите немного.");
    Ok(())
}

struct Host {
    remote: String,
    redis: String,
}

impl Host {
    fn new(remote: &str) -> Self {
        Self {
            remote: remote.to_owned(),
            redis: format!("voice-test-{}", Uuid::now_v7()),
        }
    }

    async fn start(&self, pool: &PgPool) -> anyhow::Result<axum::Router> {
        let mut st = state(pool, soundcloud_hits(), &redis_url()).await?;
        let redis = deadpool_redis::Config::from_url(redis_url())
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
        st.remote = remote_hub(&self.remote).await?;
        st.confirm = std::sync::Arc::new(LinkConfirm::new(redis, self.redis.clone()));
        Ok(crate::router::build(st))
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_desktop_polling_another_host_confirms_a_link_made_on_star(
    pool: PgPool,
) -> anyhow::Result<()> {
    let core = remote_prefix();
    let star = Host::new(&core).start(&pool).await?;
    let main = Host::new(&core).start(&pool).await?;
    let session = session_for(&pool, "101").await?;
    let confirmer = confirming_desktop(&main, session, "pc", true).await?;

    let code = new_code(&main, session, "alice").await?;
    let approved = authorize(&star, &code, "s").await?;
    assert_eq!(approved.status, StatusCode::FOUND, "{}", approved.text());
    let token = exchange(&star, &auth_code_of(&approved)).await?;
    assert!(!token.is_empty());
    confirmer.abort();
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_page_link_survives_a_star_restart_mid_confirm(pool: PgPool) -> anyhow::Result<()> {
    let core = remote_prefix();
    let star = Host::new(&core);
    let before = star.start(&pool).await?;
    let main = Host::new(&core).start(&pool).await?;
    let session = session_for(&pool, "101").await?;

    let handshake = json_call(
        &main,
        session,
        "GET",
        "/remote/wait?device=pc&active=5",
        None,
    )
    .await?
    .json();
    let code = new_code(&main, session, "alice").await?;
    let pending = submit_code(&before, &code).await?;
    assert!(pending.text().contains("Подтвердите на компьютере"));
    drop(before);

    let shown = json_call(
        &main,
        session,
        "GET",
        &format!(
            "/remote/wait?device=pc&active=5&since={}",
            handshake["cursor"].as_i64().unwrap_or_default()
        ),
        None,
    )
    .await?
    .json();
    let command = &shown["commands"][0];
    assert_eq!(command["kind"], "link_confirm");
    let answered = json_call(
        &main,
        session,
        "POST",
        "/remote/reply",
        Some(json!({"id": command["id"], "ok": true, "text": "done", "state": "done"})),
    )
    .await?;
    assert_eq!(
        answered.status,
        StatusCode::NO_CONTENT,
        "{}",
        answered.text()
    );

    let after = star.start(&pool).await?;
    let approved = ticket_flow(&after, pending).await?;
    assert_eq!(approved.status, StatusCode::FOUND, "{}", approved.text());
    exchange(&after, &auth_code_of(&approved)).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn salut_binds_after_a_star_restart_with_the_desktop_on_another_host(
    pool: PgPool,
) -> anyhow::Result<()> {
    let core = remote_prefix();
    let star = Host::new(&core);
    let before = star.start(&pool).await?;
    let main = Host::new(&core).start(&pool).await?;
    let session = session_for(&pool, "303").await?;
    let sub = "11111111-2222-3333-4444-555555555555";
    let confirmer = confirming_desktop(&main, session, "pc", true).await?;
    let code = new_code(&main, session, "salut").await?;
    let spoken = code
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let first = salut_call(
        &before,
        Some("salut-token"),
        salut_body(sub, &spoken, &spoken, 1),
    )
    .await?
    .json();
    assert!(
        first["payload"]["pronounceText"]
            .as_str()
            .unwrap_or_default()
            .contains("Подтвердите")
    );
    drop(before);

    let after = star.start(&pool).await?;
    let said = salut_until(&after, sub, "а", 2, |said| !said.contains("Подтвердите")).await?;
    confirmer.abort();
    assert!(said.contains("аккаунт связан"), "{said}");
    let bound: i64 =
        sqlx::query_scalar("SELECT count(*) FROM remote_grants WHERE external_id IS NOT NULL")
            .fetch_one(&pool)
            .await?;
    assert_eq!(bound, 1);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn remote_endpoints_answer_busy_when_nats_is_down_stays_fast(
    pool: PgPool,
) -> anyhow::Result<()> {
    let mut st = state(&pool, soundcloud_hits(), &redis_url()).await?;
    let dead = NatsService::connect(
        "nats://127.0.0.1:1",
        tokio_util::sync::CancellationToken::new(),
    )
    .await?;
    st.remote = crate::modules::remote::RemoteHub::new(Some(dead), &remote_prefix());
    let app = crate::router::build(st);
    let session = session_for(&pool, "101").await?;
    let started = std::time::Instant::now();
    for _ in 0..5 {
        let waited = json_call(
            &app,
            session,
            "GET",
            "/remote/wait?device=pc&active=1&since=0",
            None,
        )
        .await?;
        assert_eq!(waited.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(waited.json()["code"], "voice_busy");
    }
    let replied = json_call(
        &app,
        session,
        "POST",
        "/remote/reply",
        Some(json!({"id": Uuid::now_v7(), "ok": true})),
    )
    .await?;
    assert_eq!(replied.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "one open attempt, not a minute of queued ones: {:?}",
        started.elapsed()
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_reply_must_answer_a_command_a_device_of_this_account_took(
    pool: PgPool,
) -> anyhow::Result<()> {
    let st = state(&pool, soundcloud_hits(), &redis_url()).await?;
    let remote = st.remote.clone();
    let app = crate::router::build(st);
    let session = session_for(&pool, "101").await?;
    let at = |offset_ms: i64| {
        uuid::Builder::from_unix_timestamp_millis((now_ms() + offset_ms) as u64, &[3; 10])
            .into_uuid()
    };
    for id in [at(-600_000), at(600_000), Uuid::new_v4(), at(-5_000)] {
        let refused = json_call(
            &app,
            session,
            "POST",
            "/remote/reply",
            Some(json!({"id": id, "ok": true})),
        )
        .await?;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{id}");
        assert_eq!(refused.json()["code"], "voice_reply_unknown");
    }

    let handshake = json_call(
        &app,
        session,
        "GET",
        "/remote/wait?device=pc&active=5",
        None,
    )
    .await?
    .json();
    let asking = tokio::spawn(async move {
        remote
            .dispatch(
                "101",
                CommandKind::Utterance,
                "alice",
                "пауза",
                std::time::Instant::now() + Duration::from_secs(5),
            )
            .await
    });
    let shown = json_call(
        &app,
        session,
        "GET",
        &format!(
            "/remote/wait?device=pc&active=5&since={}",
            handshake["cursor"]
        ),
        None,
    )
    .await?
    .json();
    let taken = json_call(
        &app,
        session,
        "POST",
        "/remote/reply",
        Some(json!({"id": shown["commands"][0]["id"], "ok": true, "speech": "готово"})),
    )
    .await?;
    assert_eq!(taken.status, StatusCode::NO_CONTENT, "{}", taken.text());
    assert!(matches!(asking.await?, Asked::Reply(reply) if reply.speech == "готово"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_page_ticket_outlives_the_confirm_window_and_still_links(
    pool: PgPool,
) -> anyhow::Result<()> {
    let st = state(&pool, soundcloud_hits(), &redis_url()).await?;
    let confirm = st.confirm.clone();
    let remote = st.remote.clone();
    let app = crate::router::build(st);
    let session = session_for(&pool, "101").await?;
    let confirmer = confirming_desktop(&app, session, "pc", true).await?;
    let code = new_code(&app, session, "alice").await?;
    let pending = submit_code(&app, &code).await?;
    let ticket = ticket_in(&pending.text()).expect("a status link on the page");
    let key = format!("ticket:{ticket}");
    assert!(
        confirm.ttl(&key).await? > 500,
        "a pending page ticket outlives the 60 s confirmation"
    );

    let mut record: PageTicket = confirm.get(&key).await?.expect("a pending ticket");
    let asked = record.confirming.clone().expect("a confirmation in flight");
    let deadline = tokio::time::Instant::now() + SETTLE_BUDGET;
    while remote.answer(&asked.user, asked.command).await.is_none() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the desktop never answered"
        );
        tokio::time::sleep(POLL_EVERY).await;
    }
    confirmer.abort();
    record.confirming = Some(Confirming {
        deadline_ms: now_ms() - 30_000,
        ..asked
    });
    confirm.put(&key, &record, 600).await?;

    let approved = ticket_flow(&app, pending).await?;
    assert_eq!(approved.status, StatusCode::FOUND, "{}", approved.text());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn the_link_page_is_limited_per_address_with_the_shipped_limits(
    pool: PgPool,
) -> anyhow::Result<()> {
    let shipped = AdmissionCfg::from_env();
    assert!(
        shipped.voice_page_ip.global >= 1_000 * shipped.voice_page_ip.per_client
            && shipped.voice_page_client.per_client >= 1_000 * shipped.voice_page_ip.per_client,
        "the shared page limits are only a backstop far above one address"
    );
    let app = app_with(
        &pool,
        tuned(|config| {
            config.admission.voice_page_ip = shipped.voice_page_ip;
            config.admission.voice_page_client = shipped.voice_page_client;
        }),
        true,
    )
    .await?;
    let guesser = random_address();
    for _ in 0..shipped.voice_page_ip.per_client {
        let tried = page_submit_from(&app, guesser, "x").await?;
        assert_eq!(tried.status, StatusCode::BAD_REQUEST, "{}", tried.text());
    }
    let limited = page_submit_from(&app, guesser, "x").await?;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    for _ in 0..20 {
        let neighbour = page_submit_from(&app, random_address(), "x").await?;
        assert_eq!(
            neighbour.status,
            StatusCode::BAD_REQUEST,
            "one address does not use up the page for the others"
        );
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn a_salut_turn_after_the_confirm_window_still_gets_the_approval(
    pool: PgPool,
) -> anyhow::Result<()> {
    let st = state(&pool, soundcloud_hits(), &redis_url()).await?;
    let confirm = st.confirm.clone();
    let remote = st.remote.clone();
    let app = crate::router::build(st);
    let session = session_for(&pool, "303").await?;
    let sub = "88888888-2222-3333-4444-555555555555";
    let confirmer = confirming_desktop(&app, session, "pc", true).await?;
    let code = new_code(&app, session, "salut").await?;
    let spoken = code
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let first = salut_call(
        &app,
        Some("salut-token"),
        salut_body(sub, &spoken, &spoken, 1),
    )
    .await?;
    assert!(said(&first).contains("Подтвердите"), "{}", first.text());

    let key = format!("salut:{sub}");
    assert!(
        confirm.ttl(&key).await? > 500,
        "a pending Salut ticket outlives the 60 s confirmation"
    );
    let mut ticket: SalutTicket = confirm.get(&key).await?.expect("a pending ticket");
    let asked = ticket.confirming.clone().expect("a confirmation in flight");
    let deadline = tokio::time::Instant::now() + SETTLE_BUDGET;
    while remote.answer(&asked.user, asked.command).await.is_none() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the desktop never answered"
        );
        tokio::time::sleep(POLL_EVERY).await;
    }
    ticket.confirming = Some(Confirming {
        deadline_ms: now_ms() - 30_000,
        ..asked
    });
    confirm.put(&key, &ticket, 600).await?;

    let linked = salut_until(&app, sub, "а", 2, |said| !said.contains("Подтвердите")).await?;
    confirmer.abort();
    assert!(linked.contains("аккаунт связан"), "{linked}");
    Ok(())
}

#[path = "router_voice_chat_tests.rs"]
mod chat;
