use std::fs;
use std::time::Duration;

use deadpool_postgres::{Config as PgConfig, Pool, PoolConfig, Runtime, SslMode, Timeouts};
use rustls::{ClientConfig, RootCertStore};
use tokio_postgres::NoTls;
use tokio_postgres_rustls::MakeRustlsConnect;
use tracing::info;
use uuid::Uuid;

use crate::config::Config;

#[derive(Debug, thiserror::Error)]
pub enum PgError {
    #[error("pool: {0}")]
    Pool(#[from] deadpool_postgres::PoolError),
    #[error("db: {0}")]
    Postgres(#[from] tokio_postgres::Error),
}

#[derive(Clone)]
pub struct PgPool {
    pool: Pool,
}

impl PgPool {
    pub fn status(&self) -> deadpool_postgres::Status {
        self.pool.status()
    }

    async fn connection(&self) -> Result<deadpool_postgres::Object, PgError> {
        let started = std::time::Instant::now();
        let taken = self.pool.get().await;
        crate::metrics::record_pool_wait(
            if taken.is_ok() { "ok" } else { "refused" },
            started.elapsed(),
        );
        Ok(taken?)
    }
}

pub struct SessionInfo {
    pub access_token: Option<String>,
    pub soundcloud_user_id: Option<String>,
}

const SESSION_QUERY: &str = r#"SELECT CASE
                                  WHEN connection.expires_at > now()
                                   AND connection.last_refresh_error_kind IS DISTINCT FROM 'token_rejected'
                                   AND connection.last_refresh_error_kind IS DISTINCT FROM 'reauthorization_required'
                                      THEN connection.access_token
                               END,
                               connection.soundcloud_user_id
                        FROM sessions AS session
                        LEFT JOIN soundcloud_connections AS connection
                            ON connection.id = session.soundcloud_connection_id
                        WHERE session.id = $1"#;

#[derive(Debug)]
pub struct CdnTrackRecord {
    pub id: String,
    pub track_urn: String,
    pub status: String,
}

fn pool_config(pool_max: usize, acquire_timeout_secs: u64) -> PoolConfig {
    let budget = Duration::from_secs(acquire_timeout_secs);
    let mut pool = PoolConfig::new(pool_max);
    pool.timeouts = Timeouts {
        wait: Some(budget),
        create: Some(budget),
        recycle: Some(budget),
    };
    pool
}

impl PgPool {
    pub async fn connect(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let mut pg = PgConfig::new();
        pg.host = Some(config.database_host.clone());
        pg.port = Some(config.database_port);
        pg.user = Some(config.database_username.clone());
        pg.password = Some(config.database_password.clone());
        pg.dbname = Some(config.database_name.clone());
        pg.pool = Some(pool_config(
            config.database_pool_max,
            config.database_acquire_timeout_secs,
        ));

        let pool = match (
            &config.database_ssl_ca,
            &config.database_ssl_cert,
            &config.database_ssl_key,
        ) {
            (None, None, None) => pg.create_pool(Some(Runtime::Tokio1), NoTls)?,
            (Some(ca_path), Some(cert_path), Some(key_path)) => {
                let mut roots = RootCertStore::empty();
                for cert in rustls_pemfile::certs(&mut fs::read(ca_path)?.as_slice()) {
                    roots.add(cert?)?;
                }

                let chain = rustls_pemfile::certs(&mut fs::read(cert_path)?.as_slice())
                    .collect::<Result<Vec<_>, _>>()?;
                if chain.is_empty() {
                    return Err(format!("no certificates in {cert_path}").into());
                }

                let key = rustls_pemfile::private_key(&mut fs::read(key_path)?.as_slice())?
                    .ok_or_else(|| format!("no private key in {key_path}"))?;

                let tls = ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_client_auth_cert(chain, key)?;

                pg.ssl_mode = Some(SslMode::Require);
                pg.create_pool(Some(Runtime::Tokio1), MakeRustlsConnect::new(tls))?
            }
            _ => {
                return Err(
                    "DATABASE_SSL_CA, DATABASE_SSL_CERT and DATABASE_SSL_KEY must be set together"
                        .into(),
                );
            }
        };

        let client = pool.get().await?;
        let schema_ready: bool = client
            .query_one(
                "SELECT EXISTS (
                     SELECT 1
                     FROM _sqlx_migrations
                     WHERE version = 70
                       AND success = true
                 )
                 AND to_regclass('soundcloud_connections') IS NOT NULL",
                &[],
            )
            .await?
            .get(0);
        if !schema_ready {
            return Err(
                "PostgreSQL schema is incompatible; apply core migrations through 0070".into(),
            );
        }
        info!(pool_max = config.database_pool_max, "PostgreSQL connected");

        Ok(Self { pool })
    }

    pub async fn get_session(&self, session_id: &str) -> Result<Option<SessionInfo>, PgError> {
        let Ok(session_id) = Uuid::parse_str(session_id) else {
            return Ok(None);
        };
        let client = self.connection().await?;
        let row = client.query_opt(SESSION_QUERY, &[&session_id]).await?;

        Ok(row.map(|r| SessionInfo {
            access_token: r.get(0),
            soundcloud_user_id: r.get(1),
        }))
    }

    pub async fn track_is_public(&self, track_urn: &str) -> Result<Option<bool>, PgError> {
        let client = self.connection().await?;
        let row = client
            .query_opt(
                "SELECT sharing = 'public' FROM tracks WHERE urn = $1",
                &[&track_urn],
            )
            .await?;
        Ok(row.map(|row| row.get(0)))
    }

    pub async fn find_cached_track(
        &self,
        track_urn: &str,
    ) -> Result<Option<CdnTrackRecord>, PgError> {
        let client = self.connection().await?;
        let row = client
            .query_opt(
                r#"SELECT id, track_urn, status
                   FROM cdn_tracks
                   WHERE track_urn = $1 AND quality = 'single' AND status = 'ok'"#,
                &[&track_urn],
            )
            .await?;
        Ok(row.as_ref().map(row_to_cdn_track))
    }

    pub async fn update_last_accessed(&self, id: &str) -> Result<(), PgError> {
        let client = self.connection().await?;
        client
            .execute(
                r#"UPDATE cdn_tracks SET last_accessed_at = NOW() WHERE id = $1::text::uuid"#,
                &[&id],
            )
            .await?;
        Ok(())
    }

    pub async fn insert_cdn_track(
        &self,
        track_urn: &str,
        cdn_path: &str,
        status: &str,
    ) -> Result<String, PgError> {
        let id = Uuid::now_v7().to_string();
        let quality = "single";
        let client = self.connection().await?;
        client
            .execute(
                r#"INSERT INTO cdn_tracks (id, track_urn, quality, cdn_path, status, created_at, updated_at, last_accessed_at)
                   VALUES ($1::text::uuid, $2, $3, $4, $5, NOW(), NOW(), NOW())
                   ON CONFLICT (track_urn, quality) DO UPDATE SET status = $5, cdn_path = $4, updated_at = NOW()"#,
                &[&id, &track_urn, &quality, &cdn_path, &status],
            )
            .await?;
        Ok(id)
    }

    pub async fn update_cdn_track_status(&self, id: &str, status: &str) -> Result<(), PgError> {
        let client = self.connection().await?;
        client
            .execute(
                r#"UPDATE cdn_tracks SET status = $2, updated_at = NOW() WHERE id = $1::text::uuid"#,
                &[&id, &status],
            )
            .await?;
        Ok(())
    }

    pub async fn get_stale_cdn_tracks(
        &self,
        older_than_days: u64,
    ) -> Result<Vec<CdnTrackRecord>, PgError> {
        let client = self.connection().await?;
        let interval = format!("{older_than_days} days");
        let rows = client
            .query(
                r#"SELECT id, track_urn, status
                   FROM cdn_tracks
                   WHERE status = 'ok'
                     AND last_accessed_at < NOW() - $1::interval
                   ORDER BY last_accessed_at ASC"#,
                &[&interval],
            )
            .await?;

        Ok(rows.iter().map(row_to_cdn_track).collect())
    }

    pub async fn get_cdn_tracks_oldest_first(
        &self,
        limit: i64,
    ) -> Result<Vec<CdnTrackRecord>, PgError> {
        let client = self.connection().await?;
        let rows = client
            .query(
                r#"SELECT id, track_urn, status
                   FROM cdn_tracks
                   WHERE status = 'ok'
                   ORDER BY last_accessed_at ASC
                   LIMIT $1"#,
                &[&limit],
            )
            .await?;

        Ok(rows.iter().map(row_to_cdn_track).collect())
    }

    pub async fn delete_cdn_track(&self, id: &str) -> Result<(), PgError> {
        let client = self.connection().await?;
        client
            .execute("DELETE FROM cdn_tracks WHERE id = $1::text::uuid", &[&id])
            .await?;
        Ok(())
    }

    pub async fn get_app_tokens(&self, exclude_token: &str) -> Result<Vec<String>, PgError> {
        let client = self.connection().await?;
        let rows = client
            .query(
                r#"SELECT token.access_token
                   FROM oauth_app_tokens AS token
                   JOIN oauth_apps AS app ON app.id = token.oauth_app_id
                   WHERE app.active
                     AND token.expires_at > NOW() + INTERVAL '30 seconds'
                     AND token.access_token <> ''
                     AND token.access_token <> $1"#,
                &[&exclude_token],
            )
            .await?;
        let mut tokens: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
        use rand::seq::SliceRandom;
        tokens.shuffle(&mut rand::thread_rng());
        Ok(tokens)
    }

    pub async fn pick_hq_upgrade_candidates(
        &self,
        limit: i64,
        retry_cooldown_sec: i64,
    ) -> Result<Vec<String>, PgError> {
        let client = self.connection().await?;
        let rows = client
            .query(
                r#"UPDATE tracks SET hq_upgrade_last_at = now(),
                       hq_upgrade_attempts = hq_upgrade_attempts + 1, updated_at = now()
                   WHERE id IN (
                       SELECT id FROM tracks
                       WHERE hq_upgrade_pending = true
                         AND (hq_upgrade_last_at IS NULL
                              OR hq_upgrade_last_at < now() - make_interval(secs => $2))
                       ORDER BY hq_upgrade_last_at NULLS FIRST, hq_upgrade_attempts
                       FOR UPDATE SKIP LOCKED
                       LIMIT $1
                   )
                   RETURNING urn"#,
                &[&limit, &(retry_cooldown_sec as f64)],
            )
            .await?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn mark_hq_upgrade_failed(&self, urn: &str) -> Result<(), PgError> {
        let client = self.connection().await?;
        client
            .execute(
                "UPDATE tracks SET hq_upgrade_last_at = now(), updated_at = now() \
                 WHERE urn = $1",
                &[&urn],
            )
            .await?;
        Ok(())
    }

    pub async fn get_trusted_duration_ms(&self, track_urn: &str) -> Result<Option<i64>, PgError> {
        let client = self.connection().await?;
        let row = client
            .query_opt(
                r#"SELECT duration_ms FROM tracks
                   WHERE urn = $1 AND needs_duration_resolve = false
                     AND duration_ms > 0 AND duration_ms <> 30000"#,
                &[&track_urn],
            )
            .await?;
        Ok(row.map(|r| r.get::<_, i32>(0) as i64))
    }

    pub async fn is_premium(&self, user_id: &str) -> Result<bool, PgError> {
        let variants = user_id_variants(user_id);
        if variants.is_empty() {
            return Ok(false);
        }
        let client = self.connection().await?;
        let now = chrono::Utc::now().timestamp();
        let row = client
            .query_opt(
                r#"SELECT 1 FROM subscriptions WHERE user_urn = ANY($1) AND exp_date > $2"#,
                &[&variants, &now],
            )
            .await?;
        Ok(row.is_some())
    }
}

fn user_id_variants(user_id: &str) -> Vec<String> {
    let trimmed = user_id.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let bare = trimmed.rsplit(':').next().unwrap_or(trimmed);
    let mut variants = vec![trimmed.to_string()];
    if bare != trimmed {
        variants.push(bare.to_string());
    }
    if !bare.is_empty() && bare.bytes().all(|b| b.is_ascii_digit()) {
        let urn = format!("soundcloud:users:{bare}");
        if urn != trimmed {
            variants.push(urn);
        }
    }
    variants.dedup();
    variants
}

fn row_to_cdn_track(row: &tokio_postgres::Row) -> CdnTrackRecord {
    CdnTrackRecord {
        id: row.get::<_, Uuid>(0).to_string(),
        track_urn: row.get(1),
        status: row.get(2),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use deadpool_postgres::{Config as PgConfig, Runtime};
    use tokio_postgres::NoTls;
    use uuid::Uuid;

    use super::{SESSION_QUERY, pool_config, user_id_variants};

    #[test]
    fn a_pool_that_cannot_serve_a_request_gives_up_instead_of_holding_it_forever() {
        let pool = pool_config(4, 7);
        assert_eq!(pool.max_size, 4);
        assert_eq!(pool.timeouts.wait, Some(Duration::from_secs(7)));
        assert_eq!(pool.timeouts.create, Some(Duration::from_secs(7)));
        assert_eq!(pool.timeouts.recycle, Some(Duration::from_secs(7)));
    }

    #[tokio::test]
    #[ignore = "requires a database with the latest API schema"]
    async fn an_exhausted_pool_fails_the_next_request_inside_its_budget()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut pg = PgConfig::new();
        pg.url = Some(std::env::var("STREAMING_SCHEMA_TEST_DATABASE_URL")?);
        pg.pool = Some(pool_config(1, 1));
        let pool = pg.create_pool(Some(Runtime::Tokio1), NoTls)?;

        let held = pool.get().await?;
        let started = Instant::now();
        let refused = pool.get().await;
        let waited = started.elapsed();

        assert!(
            refused.is_err(),
            "the only connection is still held, so the second request cannot have been served"
        );
        assert!(
            waited < Duration::from_secs(5),
            "waited {waited:?} for a connection that was never going to come; \
             without a wait budget this request would hang until the client gave up"
        );
        drop(held);
        assert!(pool.get().await.is_ok());
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a database with the latest API schema"]
    async fn session_query_matches_latest_api_schema() -> Result<(), Box<dyn std::error::Error>> {
        let database_url = std::env::var("STREAMING_SCHEMA_TEST_DATABASE_URL")?;
        let (mut client, connection) = tokio_postgres::connect(&database_url, NoTls).await?;
        tokio::spawn(connection);

        let transaction = client.transaction().await?;
        let connection_id = Uuid::now_v7();
        let session_id = Uuid::now_v7();
        transaction
            .execute(
                "INSERT INTO soundcloud_connections (
                     id, soundcloud_user_id, access_token, refresh_token, expires_at, scope
                 ) VALUES ($1, '42', 'access', 'refresh', now() + interval '1 hour', '')",
                &[&connection_id],
            )
            .await?;
        transaction
            .execute(
                "INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)",
                &[&session_id, &connection_id],
            )
            .await?;

        let ready = transaction.query_one(SESSION_QUERY, &[&session_id]).await?;
        assert_eq!(ready.get::<_, Option<String>>(0).as_deref(), Some("access"));
        assert_eq!(ready.get::<_, Option<String>>(1).as_deref(), Some("42"));

        transaction
            .execute(
                "UPDATE soundcloud_connections
                 SET last_refresh_error_kind = 'reauthorization_required'
                 WHERE id = $1",
                &[&connection_id],
            )
            .await?;
        let rejected = transaction.query_one(SESSION_QUERY, &[&session_id]).await?;
        assert_eq!(rejected.get::<_, Option<String>>(0), None);
        assert_eq!(rejected.get::<_, Option<String>>(1).as_deref(), Some("42"));

        transaction.rollback().await?;
        Ok(())
    }

    #[test]
    fn premium_lookup_matches_bare_and_urn_user_ids() {
        assert_eq!(
            user_id_variants("12345"),
            vec!["12345", "soundcloud:users:12345"]
        );
        assert_eq!(
            user_id_variants("soundcloud:users:12345"),
            vec!["soundcloud:users:12345", "12345"]
        );
    }

    #[test]
    fn premium_lookup_trims_and_rejects_empty_ids() {
        assert_eq!(
            user_id_variants(" 12345 "),
            vec!["12345", "soundcloud:users:12345"]
        );
        assert!(user_id_variants("   ").is_empty());
    }
}
