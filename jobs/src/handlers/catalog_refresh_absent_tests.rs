use std::sync::Arc;

use backend_contracts::{CatalogEntity, CatalogRefreshPayload, CollectionFailure, JobKind};
use base64::Engine;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::CatalogRefreshHandler;
use crate::handlers::catalog_read::PublicCatalogReader;
use crate::handlers::catalog_refresh_writer::CatalogWriter;
use crate::handlers::catalog_remote::CatalogRemote;
use crate::queue::{JobError, JobRepository, LeasedJob};

const APIV2: &str = "https://api-v2.soundcloud.com/";
const HOME: &str = r#"<script>window.__sc_hydration = [{"hydratable":"apiClient","data":{"id":"stub-client"}}]</script>"#;

struct SoundCloud {
    proxy_url: String,
    asked: Arc<Mutex<Vec<String>>>,
    server: JoinHandle<anyhow::Result<()>>,
}

impl SoundCloud {
    async fn answering(answers: Vec<(&'static str, String)>) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let proxy_url = format!("http://{}/", listener.local_addr()?);
        let asked = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&asked);
        let server: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            let mut answers = answers.into_iter();
            loop {
                let (mut stream, _) = listener.accept().await?;
                let target = read_target(&mut stream).await?;
                if !target.starts_with(APIV2) {
                    respond(&mut stream, "200 OK", HOME).await?;
                    continue;
                }
                seen.lock().await.push(target);
                let (status, body) = answers
                    .next()
                    .unwrap_or_else(|| ("404 Not Found", "{}".to_owned()));
                respond(&mut stream, status, &body).await?;
            }
        });
        Ok(Self {
            proxy_url,
            asked,
            server,
        })
    }

    async fn asked(&self) -> usize {
        self.asked.lock().await.len()
    }

    fn reader(&self, pool: &PgPool) -> anyhow::Result<PublicCatalogReader> {
        let sc = sc_transport::ScClient::new(&sc_transport::ScConfig {
            proxy_url: self.proxy_url.clone(),
            proxy_fallback: false,
            api_base: Some("http://127.0.0.1:1".to_owned()),
            home_base: Some("https://soundcloud.com/".to_owned()),
        })?;
        Ok(PublicCatalogReader::new(sc, pool.clone()))
    }
}

impl Drop for SoundCloud {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn read_target(stream: &mut TcpStream) -> anyhow::Result<String> {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await?;
        if read == 0 || request.len().saturating_add(read) > 16 * 1024 {
            return Err(anyhow::anyhow!("the stub received an invalid request"));
        }
        request.extend_from_slice(&chunk[..read]);
    }
    let request = String::from_utf8(request)?;
    let encoded = request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("x-target"))
        .map(|(_, value)| value.trim())
        .ok_or_else(|| anyhow::anyhow!("the request names no target"))?;
    Ok(String::from_utf8(
        base64::engine::general_purpose::STANDARD.decode(encoded)?,
    )?)
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) -> anyhow::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

fn refresher(pool: &PgPool, soundcloud: &SoundCloud) -> anyhow::Result<CatalogRefreshHandler> {
    Ok(CatalogRefreshHandler {
        writer: CatalogWriter::new(pool.clone(), 420_000),
        public: Arc::new(soundcloud.reader(pool)?),
        remote: CatalogRemote::offline(pool.clone())?,
        pool: pool.clone(),
    })
}

fn track() -> CatalogRefreshPayload {
    CatalogRefreshPayload {
        entity: CatalogEntity::Track,
        sc_id: "42".into(),
        owner_id: None,
    }
}

async fn leased(pool: &PgPool) -> anyhow::Result<LeasedJob> {
    let job = LeasedJob {
        id: Uuid::now_v7(),
        kind: JobKind::CatalogRefresh,
        dedup_key: Some(track().dedup_key()),
        payload: Value::Null,
        generation: 1,
        attempts: 1,
        max_attempts: 8,
        lease_id: Uuid::now_v7(),
    };
    sqlx::query(
        "INSERT INTO background_jobs (
             id, kind, lane, dedup_key, payload, lease_id, lease_generation, generation,
             lease_expires_at, leased_by, attempts
         ) VALUES (
             $1, 'catalog.refresh', 'core_bulk', $2, '{}'::jsonb, $3, 1, 1,
             now() + interval '1 minute', 'refresh-test', 1
         )",
    )
    .bind(job.id)
    .bind(&job.dedup_key)
    .bind(job.lease_id)
    .execute(pool)
    .await?;
    Ok(job)
}

async fn refresh(pool: &PgPool, handler: &CatalogRefreshHandler) -> anyhow::Result<()> {
    let job = leased(pool).await?;
    match handler.refresh(&job, track()).await {
        Err(error @ JobError::Settled { .. }) => {
            JobRepository::new(pool.clone(), "refresh-test".to_owned())
                .fail(&job, &error.to_string(), false)
                .await?;
            Ok(())
        }
        Err(error) => Err(anyhow::anyhow!("refresh failed unsettled: {error}")),
        Ok(()) => {
            JobRepository::new(pool.clone(), "refresh-test".to_owned())
                .complete(&job)
                .await?;
            Ok(())
        }
    }
}

async fn settled(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT last_error FROM background_job_failures
         WHERE kind = 'catalog.refresh' AND dedup_key = 'track:42:public'
         ORDER BY failed_at",
    )
    .fetch_all(pool)
    .await?)
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_track_absent_in_every_region_settles_once_and_is_held_without_asking_again(
    pool: PgPool,
) -> anyhow::Result<()> {
    let soundcloud = SoundCloud::answering(Vec::new()).await?;
    let handler = refresher(&pool, &soundcloud)?;

    let job = leased(&pool).await?;
    let error = handler
        .refresh(&job, track())
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("an absent track cannot be stored"))?;
    assert!(matches!(
        error,
        JobError::Settled {
            failure: CollectionFailure::NotFound,
            ..
        }
    ));
    assert_eq!(soundcloud.asked().await, 3);
    JobRepository::new(pool.clone(), "refresh-test".to_owned())
        .fail(&job, &error.to_string(), false)
        .await?;

    refresh(&pool, &handler).await?;
    refresh(&pool, &handler).await?;

    assert_eq!(soundcloud.asked().await, 3);
    assert_eq!(
        settled(&pool).await?,
        vec!["not_found: SoundCloud API error (status 404)".to_owned()]
    );
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(queued, 0);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_held_absence_is_asked_again_once_its_day_is_over(pool: PgPool) -> anyhow::Result<()> {
    let soundcloud = SoundCloud::answering(Vec::new()).await?;
    let handler = refresher(&pool, &soundcloud)?;
    refresh(&pool, &handler).await?;
    sqlx::query("UPDATE background_job_failures SET failed_at = now() - interval '25 hours'")
        .execute(&pool)
        .await?;

    refresh(&pool, &handler).await?;

    assert_eq!(soundcloud.asked().await, 6);
    assert_eq!(settled(&pool).await?.len(), 2);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_track_one_region_hides_is_stored_and_never_settled(pool: PgPool) -> anyhow::Result<()> {
    let found = json!({
        "id": 42, "urn": "soundcloud:tracks:42", "title": "Found", "sharing": "public",
        "duration": 120000, "user": {"id": 7, "urn": "soundcloud:users:7", "username": "Owner"}
    });
    let soundcloud = SoundCloud::answering(vec![
        ("404 Not Found", "{}".to_owned()),
        ("200 OK", found.to_string()),
    ])
    .await?;
    let handler = refresher(&pool, &soundcloud)?;

    refresh(&pool, &handler).await?;

    assert_eq!(soundcloud.asked().await, 2);
    assert!(settled(&pool).await?.is_empty());
    let title: String = sqlx::query_scalar("SELECT title FROM tracks WHERE sc_track_id = '42'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(title, "Found");
    Ok(())
}
