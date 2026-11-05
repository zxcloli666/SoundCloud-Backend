use anyhow::ensure;
use sqlx::PgPool;

pub const OWNER: &str = "chat_auth";
pub const RELEASE_OWNER: &str = "chat_auth_release";
pub const RELEASE: &str = "public.auth_connection_release(uuid)";

pub const CHAT_FUNCTIONS: [&str; 5] = [
    "public.auth_session(uuid)",
    "public.auth_sessions_alive(bytea[])",
    "public.auth_session_logged_out(uuid)",
    "public.auth_session_username(uuid)",
    "public.auth_session_end(uuid, bytea)",
];

pub async fn verify(pool: &PgPool) -> anyhow::Result<()> {
    let mut signatures = CHAT_FUNCTIONS.to_vec();
    let mut owners = vec![OWNER; CHAT_FUNCTIONS.len()];
    let mut called = vec![true; CHAT_FUNCTIONS.len()];
    signatures.push(RELEASE);
    owners.push(RELEASE_OWNER);
    called.push(false);
    let wrong: Vec<String> = sqlx::query_scalar(
        "SELECT expected.signature
         FROM unnest($1::text[], $2::text[], $3::bool[]) AS expected(signature, owner, called)
         WHERE NOT EXISTS (
             SELECT 1
             FROM pg_catalog.pg_proc AS routine
             WHERE routine.oid = to_regprocedure(expected.signature)
               AND routine.prosecdef
               AND routine.proowner = to_regrole(expected.owner)
               AND (NOT expected.called OR has_function_privilege(routine.oid, 'EXECUTE'))
         )",
    )
    .bind(&signatures)
    .bind(&owners)
    .bind(&called)
    .fetch_all(pool)
    .await?;
    ensure!(
        wrong.is_empty(),
        "auth functions must be SECURITY DEFINER under their NOLOGIN owners and executable by this role: {}",
        wrong.join(", ")
    );
    Ok(())
}
