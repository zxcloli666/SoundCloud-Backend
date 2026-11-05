use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use crate::modules::auth::definers::CHAT_FUNCTIONS;

pub struct Restricted {
    pub role: String,
    pub pool: PgPool,
}

impl Restricted {
    pub async fn new(owner: &PgPool) -> anyhow::Result<Self> {
        let role = format!("chat_{}", Uuid::new_v4().simple());
        let password = Uuid::new_v4().simple().to_string();
        let functions = CHAT_FUNCTIONS.join(", ");
        sqlx::raw_sql(&format!(
            "CREATE ROLE {role} LOGIN PASSWORD '{password}'
                 NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOREPLICATION;
             GRANT USAGE ON SCHEMA chat TO {role};
             GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA chat TO {role};
             GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA chat TO {role};
             GRANT SELECT ON public._sqlx_migrations, public.users, public.user_followings,
                 public.user_blocked_artists, public.subscriptions TO {role};
             REVOKE ALL ON public.sessions, public.login_requests, public.link_requests,
                 public.soundcloud_connections, public.repl_tombstones,
                 public.sync_queue_action_pauses, public.oauth_apps,
                 public.oauth_app_request_cooldowns FROM {role};
             REVOKE CREATE ON SCHEMA public FROM PUBLIC;
             REVOKE ALL ON SEQUENCE public.scd_orphan_sessions FROM {role};
             GRANT EXECUTE ON FUNCTION {functions} TO {role};"
        ))
        .execute(owner)
        .await?;
        let options = owner
            .connect_options()
            .as_ref()
            .clone()
            .username(&role)
            .password(&password);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        Ok(Self { role, pool })
    }

    pub async fn remove(self, owner: &PgPool) -> anyhow::Result<()> {
        self.pool.close().await;
        let role = self.role;
        sqlx::raw_sql(&format!("DROP OWNED BY {role}; DROP ROLE {role};"))
            .execute(owner)
            .await?;
        Ok(())
    }
}
