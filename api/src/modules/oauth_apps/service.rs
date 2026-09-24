use std::sync::Arc;

use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::oauth_apps::model::OAuthApp;

pub struct OAuthAppsService {
    pool: PgPool,
}

impl OAuthAppsService {
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }

    pub async fn count_active(&self) -> AppResult<i64> {
        let n: i64 = sqlx::query_file_scalar!("queries/oauth_apps/service/count_active.sql")
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    pub async fn pick_lru_from(&self, ids: &[Uuid]) -> AppResult<OAuthApp> {
        if ids.is_empty() {
            return Err(AppError::service_unavailable(
                "No active OAuth apps available",
            ));
        }
        let app = sqlx::query_file_as!(
            OAuthApp,
            "queries/oauth_apps/service/pick_lru_from.sql",
            ids
        )
        .fetch_optional(&self.pool)
        .await?;
        let app = match app {
            Some(app) => Some(app),
            None => {
                sqlx::query_file_as!(
                    OAuthApp,
                    "queries/oauth_apps/service/pick_lru_wait.sql",
                    ids
                )
                .fetch_optional(&self.pool)
                .await?
            }
        };

        let app =
            app.ok_or_else(|| AppError::service_unavailable("No active OAuth apps available"))?;
        info!(app_name = %app.name, app_id = %app.id, "Picked OAuth app — lastUsedAt updated");
        Ok(app)
    }

    pub async fn get_by_id(&self, id: &str) -> AppResult<Option<OAuthApp>> {
        let uuid = match Uuid::parse_str(id) {
            Ok(u) => u,
            Err(_) => return Ok(None),
        };
        let row: Option<OAuthApp> =
            sqlx::query_file_as!(OAuthApp, "queries/oauth_apps/service/get_by_id.sql", uuid)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row)
    }

    pub async fn find_all(&self) -> AppResult<Vec<OAuthApp>> {
        let rows: Vec<OAuthApp> =
            sqlx::query_file_as!(OAuthApp, "queries/oauth_apps/service/find_all.sql")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }

    pub async fn create(
        &self,
        name: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
        active: Option<bool>,
    ) -> AppResult<OAuthApp> {
        let name = normalized_non_blank("name", name)?;
        let client_id = normalized_non_blank("clientId", client_id)?;
        let client_secret = non_blank_secret(client_secret)?;
        let redirect_uri = normalized_redirect_uri(redirect_uri)?;
        let row: OAuthApp = sqlx::query_file_as!(
            OAuthApp,
            "queries/oauth_apps/service/create.sql",
            Uuid::now_v7(),
            &name,
            &client_id,
            client_secret,
            &redirect_uri,
            active.unwrap_or(true)
        )
        .fetch_one(&self.pool)
        .await
        .map_err(oauth_app_write_error)?;
        Ok(row)
    }

    pub async fn update(
        &self,
        id: &str,
        name: Option<&str>,
        client_id: Option<&str>,
        client_secret: Option<&str>,
        redirect_uri: Option<&str>,
        active: Option<bool>,
    ) -> AppResult<OAuthApp> {
        let uuid = Uuid::parse_str(id).map_err(|_| AppError::not_found("OAuth app not found"))?;
        let current = self
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::not_found("OAuth app not found"))?;
        if client_id.is_some_and(|value| value.trim() != current.client_id.trim()) {
            return Err(AppError::bad_request(
                "clientId is immutable; create a new OAuth app",
            ));
        }
        let name = name
            .map(|value| normalized_non_blank("name", value))
            .transpose()?;
        let client_secret = client_secret.map(non_blank_secret).transpose()?;
        let redirect_uri = redirect_uri.map(normalized_redirect_uri).transpose()?;
        let row: Option<OAuthApp> = sqlx::query_file_as!(
            OAuthApp,
            "queries/oauth_apps/service/update.sql",
            uuid,
            name.as_deref(),
            client_secret,
            redirect_uri.as_deref(),
            active
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(oauth_app_write_error)?;
        row.ok_or_else(|| AppError::not_found("OAuth app not found"))
    }

    pub async fn remove(&self, id: &str) -> AppResult<()> {
        let uuid = match Uuid::parse_str(id) {
            Ok(u) => u,
            Err(_) => return Ok(()),
        };
        sqlx::query_file!("queries/oauth_apps/service/delete_by_id.sql", uuid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn normalized_non_blank(field: &str, value: &str) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::bad_request(format!("{field} must not be blank")));
    }
    Ok(value.to_owned())
}

fn non_blank_secret(value: &str) -> AppResult<&str> {
    if value.trim().is_empty() {
        return Err(AppError::bad_request("clientSecret must not be blank"));
    }
    Ok(value)
}

fn normalized_redirect_uri(value: &str) -> AppResult<String> {
    let value = normalized_non_blank("redirectUri", value)?;
    let uri = url::Url::parse(&value)
        .map_err(|_| AppError::bad_request("redirectUri must be a valid URL"))?;
    if !matches!(uri.scheme(), "http" | "https") {
        return Err(AppError::bad_request("redirectUri must use HTTP or HTTPS"));
    }
    Ok(value)
}

fn oauth_app_write_error(error: sqlx::Error) -> AppError {
    if error
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
    {
        return AppError::bad_request("clientId already belongs to another OAuth app");
    }
    AppError::Db(error)
}

#[cfg(test)]
mod tests {
    use futures::future::join_all;
    use sqlx::PgPool;

    use super::*;

    async fn install_oauth_apps(pool: &PgPool) -> anyhow::Result<Uuid> {
        sqlx::query(
            "CREATE TABLE oauth_apps (
                id uuid PRIMARY KEY,
                name text NOT NULL,
                client_id text NOT NULL,
                client_secret text NOT NULL,
                redirect_uri text NOT NULL,
                active boolean NOT NULL DEFAULT true,
                last_used_at timestamptz,
                created_at timestamp NOT NULL DEFAULT now(),
                updated_at timestamp NOT NULL DEFAULT now()
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"CREATE UNIQUE INDEX oauth_apps_client_identity_uq
             ON oauth_apps (
                 btrim(
                     client_id,
                     U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
                 )
             )",
        )
        .execute(pool)
        .await?;
        let app_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
             VALUES ($1, 'primary', 'client', 'secret', 'http://localhost/callback')",
        )
        .bind(app_id)
        .execute(pool)
        .await?;
        Ok(app_id)
    }

    #[sqlx::test(migrations = false)]
    async fn picker_waits_for_the_only_locked_app(pool: PgPool) -> anyhow::Result<()> {
        let app_id = install_oauth_apps(&pool).await?;
        let mut lock = pool.begin().await?;
        sqlx::query("SELECT id FROM oauth_apps WHERE id = $1 FOR UPDATE")
            .bind(app_id)
            .fetch_one(&mut *lock)
            .await?;
        let service = OAuthAppsService::new(pool);
        let picker = tokio::spawn(async move { service.pick_lru_from(&[app_id]).await });

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        lock.commit().await?;
        let picked = picker.await??;

        assert_eq!(picked.id, app_id);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn concurrent_picks_share_the_only_active_app(pool: PgPool) -> anyhow::Result<()> {
        let app_id = install_oauth_apps(&pool).await?;
        let service = OAuthAppsService::new(pool);
        let picks = (0..32).map(|_| {
            let service = Arc::clone(&service);
            async move { service.pick_lru_from(&[app_id]).await }
        });

        let picked = join_all(picks)
            .await
            .into_iter()
            .collect::<AppResult<Vec<_>>>()?;

        assert!(picked.iter().all(|app| app.id == app_id));
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn picker_returns_service_unavailable_without_an_active_app(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let app_id = install_oauth_apps(&pool).await?;
        sqlx::query("UPDATE oauth_apps SET active = false WHERE id = $1")
            .bind(app_id)
            .execute(&pool)
            .await?;
        let service = OAuthAppsService::new(pool);

        let error = service.pick_lru_from(&[app_id]).await.unwrap_err();

        assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn client_identity_cannot_change_in_place(pool: PgPool) -> anyhow::Result<()> {
        let app_id = install_oauth_apps(&pool).await?;
        let service = OAuthAppsService::new(pool.clone());

        let error = service
            .update(
                &app_id.to_string(),
                None,
                Some("another-client"),
                None,
                None,
                None,
            )
            .await
            .expect_err("client identity must be immutable");
        let client_id: String =
            sqlx::query_scalar("SELECT client_id FROM oauth_apps WHERE id = $1")
                .bind(app_id)
                .fetch_one(&pool)
                .await?;

        assert!(matches!(error, AppError::BadRequest(_)));
        assert_eq!(client_id, "client");
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn normalized_client_identity_is_unique(pool: PgPool) -> anyhow::Result<()> {
        install_oauth_apps(&pool).await?;
        let service = OAuthAppsService::new(pool);

        let error = service
            .create(
                "duplicate",
                " client ",
                "secret",
                "https://example.com/callback",
                None,
            )
            .await
            .expect_err("normalized duplicate must be rejected");

        assert!(matches!(error, AppError::BadRequest(_)));
        Ok(())
    }
}
