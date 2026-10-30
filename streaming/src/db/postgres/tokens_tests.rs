use crate::db::postgres::test_nodes::core_and_api;

const APPS: &str = "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri, active) VALUES
       ('00000000-0000-4000-8000-00000000000a', 'shared', 'client-a', 'secret-a', 'https://a.test/cb', true),
       ('00000000-0000-4000-8000-00000000000b', 'local', 'client-b', 'secret-b', 'https://b.test/cb', true),
       ('00000000-0000-4000-8000-00000000000c', 'retired', 'client-c', 'secret-c', 'https://c.test/cb', false),
       ('00000000-0000-4000-8000-00000000000d', 'stale', 'client-d', 'secret-d', 'https://d.test/cb', true);
     INSERT INTO oauth_app_tokens (oauth_app_id, access_token, expires_at)
     VALUES ('00000000-0000-4000-8000-00000000000a', 'token-shared', now() + interval '1 hour');
     INSERT INTO oauth_app_tokens_local (oauth_app_id, access_token, expires_at) VALUES
       ('00000000-0000-4000-8000-00000000000a', 'token-shared', now() + interval '1 hour'),
       ('00000000-0000-4000-8000-00000000000b', 'token-local', now() + interval '1 hour'),
       ('00000000-0000-4000-8000-00000000000c', 'token-retired', now() + interval '1 hour'),
       ('00000000-0000-4000-8000-00000000000d', 'token-stale', now() + interval '10 seconds');";

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn app_tokens_come_from_the_shared_and_the_node_table() {
    let (core, api) = core_and_api(APPS).await;

    let mut tokens = api.pg.get_app_tokens("").await.unwrap();
    tokens.sort();

    assert_eq!(tokens, ["token-local", "token-shared"]);
    core.finish().await;
    api.finish().await;
}

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn the_token_that_failed_is_not_offered_again() {
    let (core, api) = core_and_api(APPS).await;

    let tokens = api.pg.get_app_tokens("token-shared").await.unwrap();

    assert_eq!(tokens, ["token-local"]);
    core.finish().await;
    api.finish().await;
}
