use crate::db::postgres::test_nodes::core_and_api;

const LINKED_SESSION: &str = "00000000-0000-4000-8000-000000000101";
const WEB_ONLY_SESSION: &str = "00000000-0000-4000-8000-000000000102";
const RELINK_SESSION: &str = "00000000-0000-4000-8000-000000000103";
const RETIRED_APP_SESSION: &str = "00000000-0000-4000-8000-000000000104";
const LINKED_CONNECTION: &str = "00000000-0000-4000-8000-000000000201";

const ACCOUNTS: &str = "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri, active, client_kind, app_id, app_version, user_agent) VALUES
       ('00000000-0000-4000-8000-00000000000e', 'mobile', 'client-e', 'secret-e', 'sc://auth', true, 'mobile', 'app.id', '1.0', 'agent'),
       ('00000000-0000-4000-8000-00000000000f', 'mobile-old', 'client-f', 'secret-f', 'sc://auth', false, 'mobile', 'app.id', '0.9', 'agent');
     INSERT INTO soundcloud_connections (id, soundcloud_user_id, access_token, refresh_token, expires_at, scope) VALUES
       ('00000000-0000-4000-8000-000000000201', '501', 'web-501', 'refresh', now() + interval '1 hour', ''),
       ('00000000-0000-4000-8000-000000000202', '502', 'web-502', 'refresh', now() + interval '1 hour', ''),
       ('00000000-0000-4000-8000-000000000203', '503', 'web-503', 'refresh', now() + interval '1 hour', ''),
       ('00000000-0000-4000-8000-000000000204', '504', 'web-504', 'refresh', now() + interval '1 hour', '');
     INSERT INTO sessions (id, soundcloud_connection_id) VALUES
       ('00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-000000000201'),
       ('00000000-0000-4000-8000-000000000102', '00000000-0000-4000-8000-000000000202'),
       ('00000000-0000-4000-8000-000000000103', '00000000-0000-4000-8000-000000000203'),
       ('00000000-0000-4000-8000-000000000104', '00000000-0000-4000-8000-000000000204');
     INSERT INTO mobile_credentials (soundcloud_user_id, oauth_app_id, device_id, access_token, refresh_token, expires_at, scope, linked_at, plan) VALUES
       ('501', '00000000-0000-4000-8000-00000000000e', '00000000-0000-4000-8000-0000000000d1', 'mobile-501', 'mobile-refresh', now() + interval '6 days', '*', now(), 'none'),
       ('503', '00000000-0000-4000-8000-00000000000e', '00000000-0000-4000-8000-0000000000d3', 'mobile-503', 'mobile-refresh', now() + interval '6 days', '*', now(), NULL),
       ('504', '00000000-0000-4000-8000-00000000000f', '00000000-0000-4000-8000-0000000000d4', 'mobile-504', 'mobile-refresh', now() + interval '6 days', '*', now(), NULL);
     UPDATE mobile_credentials SET status = 'relink_required' WHERE soundcloud_user_id = '503';";

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn a_linked_account_brings_its_mobile_profile_with_the_session() {
    let (core, api) = core_and_api(ACCOUNTS).await;

    let session = api.pg.get_session(LINKED_SESSION).await.unwrap().unwrap();

    assert_eq!(session.access_token.as_deref(), Some("web-501"));
    let mobile = session.mobile.expect("an active link with an active app");
    assert_eq!(mobile.connection_id.to_string(), LINKED_CONNECTION);
    assert_eq!(mobile.account, "501");
    assert_eq!(mobile.access_token.expose(), "mobile-501");
    assert!(mobile.fresh);
    assert_eq!(mobile.device_id, "000000000000400080000000000000d1");
    assert_eq!(mobile.plan.as_deref(), Some("none"));
    assert_eq!(mobile.app.client_id, "client-e");
    assert_eq!(mobile.app.app_version, "1.0");
    assert!(
        mobile.app.client_secret.is_empty(),
        "streaming never needs the app secret, so the query never reads it"
    );
    core.finish().await;
    api.finish().await;
}

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn a_session_without_a_usable_link_streams_as_before() {
    let (core, api) = core_and_api(ACCOUNTS).await;

    for session_id in [WEB_ONLY_SESSION, RELINK_SESSION, RETIRED_APP_SESSION] {
        let session = api.pg.get_session(session_id).await.unwrap().unwrap();

        assert!(session.access_token.is_some(), "{session_id}");
        assert!(session.mobile.is_none(), "{session_id}");
    }
    core.finish().await;
    api.finish().await;
}

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn an_expiring_mobile_token_is_handed_over_as_stale() {
    let (core, api) = core_and_api(ACCOUNTS).await;
    api.sql
        .batch_execute(
            "UPDATE mobile_credentials SET expires_at = now() + interval '20 seconds'
             WHERE soundcloud_user_id = '501'",
        )
        .await
        .unwrap();

    let session = api.pg.get_session(LINKED_SESSION).await.unwrap().unwrap();

    assert!(session.mobile.is_some_and(|mobile| !mobile.fresh));
    core.finish().await;
    api.finish().await;
}

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn a_new_plan_is_written_once_and_versioned_for_replication() {
    let (core, api) = core_and_api(ACCOUNTS).await;
    let before: i64 = api
        .value("SELECT plan_v FROM mobile_credentials WHERE soundcloud_user_id = '501'")
        .await;

    let first = api.pg.note_mobile_plan("501", "high_tier").await.unwrap();
    let again = api.pg.note_mobile_plan("501", "high_tier").await.unwrap();

    assert!(first);
    assert!(
        !again,
        "the same plan is no write, so replication carries nothing"
    );
    let plan: String = api
        .value("SELECT plan FROM mobile_credentials WHERE soundcloud_user_id = '501'")
        .await;
    let after: i64 = api
        .value("SELECT plan_v FROM mobile_credentials WHERE soundcloud_user_id = '501'")
        .await;
    let seen: bool = api
        .value("SELECT plan_seen_at IS NOT NULL FROM mobile_credentials WHERE soundcloud_user_id = '501'")
        .await;
    assert_eq!(plan, "high_tier");
    assert!(after > before, "plan_v {before} -> {after}");
    assert!(seen);
    core.finish().await;
    api.finish().await;
}
