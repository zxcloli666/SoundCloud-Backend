use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::router_search_tests::{app, app_without_star, redis_url};

const REDIRECT: &str = "https://social.yandex.net/broker/redirect";

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
    let response = app.clone().oneshot(request.body(body)?).await?;
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
    let mut reply = send(
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
    for _ in 0..200 {
        let text = reply.text();
        let Some(at) = text.find("/oauth/authorize/status?ticket=") else {
            return Ok(reply);
        };
        let ticket: String = text[at + "/oauth/authorize/status?ticket=".len()..]
            .chars()
            .take_while(|c| *c != '"')
            .collect();
        tokio::time::sleep(Duration::from_millis(100)).await;
        reply = send(
            app,
            "GET",
            &format!("/oauth/authorize/status?ticket={ticket}"),
            &[],
            None,
        )
        .await?;
    }
    Ok(reply)
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
            "projectName": "lolya",
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
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

    let replay = token_call(&app, &exchange).await?;
    assert_eq!(replay.status, StatusCode::BAD_REQUEST);
    assert_eq!(replay.json()["error"], "invalid_grant");

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

    sqlx::query("UPDATE remote_grants SET rotated_at = now() - interval '2 minutes'")
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
            ("refresh_token", next["refresh_token"].as_str().unwrap_or_default()),
            ("client_id", "yandex"),
            ("client_secret", "s3cret"),
        ],
    )
    .await?;
    assert_eq!(current.status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_code_for_one_provider_does_not_link_the_other(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let code = new_code(&app, session, "salut").await?;
    let wrong = authorize(&app, &code, "s").await?;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
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

    let after = alice_call(&app, Some(&access), alice_body("помощь", true))
        .await?
        .json();
    assert!(after["response"]["directives"]["start_account_linking"].is_object());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
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
    let started = std::time::Instant::now();
    let answered = alice_call(&app, Some(&access), alice_body("следующий трек", true))
        .await?
        .json();
    assert_eq!(answered["response"]["text"], "Включаю следующий трек");
    assert_eq!(answered["response"]["tts"], "Включаю следующий трек");
    assert_eq!(answered["response"]["end_session"], true);
    assert!(started.elapsed() < Duration::from_secs(2));
    let delivered = desktop.await?;
    assert_eq!(delivered["commands"][0]["text"], "следующий трек");
    assert_eq!(delivered["commands"][0]["source"], "alice");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
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
    let mut linked = Value::Null;
    for attempt in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        linked = salut_call(
            &app,
            Some("salut-token"),
            salut_body(sub, "ну что", "ну что", 20 + attempt),
        )
        .await?
        .json();
        if linked["payload"]["pronounceText"]
            .as_str()
            .unwrap_or_default()
            .contains("аккаунт связан")
        {
            break;
        }
    }
    confirmer.abort();
    assert!(
        linked["payload"]["pronounceText"]
            .as_str()
            .unwrap_or_default()
            .contains("аккаунт связан"),
        "{linked}"
    );

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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn resolve_merges_variants_and_matches_case_forms(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, track_count_primary, monthly_listeners)
         VALUES ('00000000-0000-0000-0000-0000000000a1', 'Земфира', 'земфира', 'test', 40, 900000),
                ('00000000-0000-0000-0000-0000000000a2', 'Земфирка Мэш', 'земфирка мэш', 'test', 3, 10)",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, metadata_artist,
                             uploader_username, uploader_sc_user_id, duration_ms, play_count_sc, sharing)
         VALUES ('9001', 'soundcloud:tracks:9001', 'Хочешь', 'хочешь', 'Земфира', 'Земфира', '555', 200000, 50000, 'public'),
                ('9002', 'soundcloud:tracks:9002', 'Хочешь (remix)', 'хочешь remix', 'Земфира', 'Земфира', '555', 210000, 10, 'public')",
    )
    .execute(&pool)
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_link_page_refuses_framing_and_waits_for_the_desktop(pool: PgPool) -> anyhow::Result<()> {
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_no_from_the_desktop_issues_nothing_and_burns_the_code(pool: PgPool) -> anyhow::Result<()> {
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn salut_binds_only_after_a_yes_and_a_no_binds_nothing(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "303").await?;
    let sub = "11111111-2222-3333-4444-555555555555";
    let confirmer = confirming_desktop(&app, session, "pc", false).await?;
    let code = new_code(&app, session, "salut").await?;
    let spoken = code.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ");
    let first = salut_call(&app, Some("salut-token"), salut_body(sub, &spoken, &spoken, 1))
        .await?
        .json();
    assert!(
        first["payload"]["pronounceText"]
            .as_str()
            .unwrap_or_default()
            .contains("Подтвердите")
    );
    let mut text = String::new();
    for n in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let next = salut_call(&app, Some("salut-token"), salut_body(sub, "а", "а", 2 + n))
            .await?
            .json();
        text = next["payload"]["pronounceText"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if !text.contains("Подтвердите") {
            break;
        }
    }
    confirmer.abort();
    assert!(text.contains("отклонена"), "{text}");
    let bound: i64 = sqlx::query_scalar("SELECT count(*) FROM remote_grants WHERE external_id IS NOT NULL")
        .fetch_one(&pool)
        .await?;
    assert_eq!(bound, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn five_unanswered_asks_burn_a_code(pool: PgPool) -> anyhow::Result<()> {
    let redis = deadpool_redis::Config::from_url(redis_url())
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let confirm = crate::modules::remote::LinkConfirm::new(redis, format!("voice-test-{}", Uuid::now_v7()));
    let links = crate::modules::remote::LinksService::new(pool.clone());
    let issued = links
        .create_code("101", backend_contracts::remote::LinkProvider::Alice)
        .await?;
    let found = links
        .find_code(backend_contracts::remote::LinkProvider::Alice, &issued.code)
        .await?
        .expect("the code exists");
    for expected in 1..5 {
        let misses = crate::modules::remote::linking::note_miss(&links, &confirm, found.id).await;
        assert_eq!(misses, expected);
        assert!(
            links
                .find_code(backend_contracts::remote::LinkProvider::Alice, &issued.code)
                .await?
                .is_some()
        );
    }
    crate::modules::remote::linking::note_miss(&links, &confirm, found.id).await;
    assert!(
        links
            .find_code(backend_contracts::remote::LinkProvider::Alice, &issued.code)
            .await?
            .is_none(),
        "the fifth miss invalidates the code"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_desktop_shapes_are_tolerated(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let first = json_call(&app, session, "GET", "/remote/wait?since=&device=pc&active=", None).await?;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert!(first.json()["cursor"].as_i64().unwrap_or_default() > 0);
    let bad = json_call(&app, session, "GET", "/remote/wait?since=abc", None).await?;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);

    let (access, _) = link_alice(&app, session).await?;
    let desktop = tokio::spawn({
        let app = app.clone();
        async move {
            let waited = json_call(
                &app,
                session,
                "GET",
                &format!("/remote/wait?device=pc&active=1&since={}", first.json()["cursor"]),
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
    tokio::time::sleep(Duration::from_millis(200)).await;
    let answered = alice_call(&app, Some(&access), alice_body("пауза", true))
        .await?
        .json();
    desktop.await?;
    assert_eq!(answered["response"]["text"], "Из поля text");
    let links = json_call(&app, session, "GET", "/remote/links", None).await?.json();
    assert!(links["links"].is_array());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn waits_per_user_are_capped_and_a_repeat_replaces_the_first(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
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
    tokio::time::sleep(Duration::from_millis(300)).await;
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn reply_is_capped_and_needs_star(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let big = json!({"id": Uuid::now_v7(), "ok": true, "state": "x".repeat(20_000)});
    let refused = json_call(&app, session, "POST", "/remote/reply", Some(big)).await?;
    assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE);
    let small = json!({"id": Uuid::now_v7(), "ok": true});
    let fine = json_call(&app, session, "POST", "/remote/reply", Some(small.clone())).await?;
    assert_eq!(fine.status, StatusCode::NO_CONTENT);

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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn resolve_survives_a_hostile_body(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let started = std::time::Instant::now();
    let reply = json_call(
        &app,
        session,
        "POST",
        "/search/resolve",
        Some(json!({
            "variants": (0..500).map(|n| format!("{}{n}", "я".repeat(300))).collect::<Vec<_>>(),
            "kinds": vec!["track"; 30_000],
        })),
    )
    .await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn alice_answers_inside_its_budget_when_the_desktop_is_silent(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let (access, _) = link_alice(&app, session).await?;
    json_call(&app, session, "GET", "/remote/wait?device=quiet&active=9", None).await?;
    let started = std::time::Instant::now();
    let reply = alice_call(&app, Some(&access), alice_body("пауза", true))
        .await?
        .json();
    let took = started.elapsed();
    assert!(reply["response"]["text"].as_str().unwrap_or_default().contains("не отвечает"));
    assert!(took >= Duration::from_millis(600) && took < Duration::from_millis(1000), "{took:?}");
    Ok(())
}
