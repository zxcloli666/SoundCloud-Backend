mod delete_tests;
mod fuzz_tests;
mod mover_tests;
mod reconcile_tests;
mod room_tests;

use std::sync::{Arc, LazyLock};

use aws_sdk_s3::Client;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use sqlx::PgPool;
use tokio::sync::{Mutex, MutexGuard};

use crate::modules::chat::ChatState;
use crate::modules::chat::media::MediaConfig;
use crate::modules::chat::sweeps::media::fixtures::{Upload, keys_dir, ok, state_with};
use crate::modules::chat::sweeps::media::{mover, reconcile};

const REDIS_DB: u8 = 8;

static SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub struct Live {
    endpoint: String,
    access_key: String,
    secret: String,
}

impl Live {
    fn from_env() -> Self {
        let var = |name: &str| {
            std::env::var(name).unwrap_or_else(|_| panic!("{name} must point at a test SeaweedFS"))
        };
        Self {
            endpoint: var("CHAT_TEST_S3_ENDPOINT"),
            access_key: var("CHAT_TEST_S3_ACCESS_KEY"),
            secret: var("CHAT_TEST_S3_SECRET_KEY"),
        }
    }

    fn client(&self) -> Client {
        let credentials = Credentials::new(&self.access_key, &self.secret, None, None, "chat-test");
        Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .region(Region::new("us-east-1"))
                .endpoint_url(&self.endpoint)
                .credentials_provider(credentials)
                .force_path_style(true)
                .build(),
        )
    }

    async fn bucket(&self, index: usize) -> anyhow::Result<String> {
        let name = format!("relsweep-test-{index}");
        let client = self.client();
        if client.head_bucket().bucket(&name).send().await.is_err() {
            client.create_bucket().bucket(&name).send().await?;
        }
        Ok(name)
    }

    async fn empty(&self, bucket: &str) -> anyhow::Result<()> {
        let client = self.client();
        loop {
            let listed = client.list_objects_v2().bucket(bucket).send().await?;
            let keys: Vec<String> = listed
                .contents()
                .iter()
                .filter_map(|object| object.key().map(str::to_owned))
                .collect();
            if keys.is_empty() {
                return Ok(());
            }
            for key in keys {
                client
                    .delete_object()
                    .bucket(bucket)
                    .key(key)
                    .send()
                    .await?;
            }
        }
    }
}

pub struct Bench {
    pub chat: Arc<ChatState>,
    pub pg: PgPool,
    _serial: MutexGuard<'static, ()>,
}

pub struct StoreSpec<'a> {
    pub id: u16,
    pub bucket: &'a str,
    pub endpoint: Option<&'a str>,
}

pub async fn bench(pg: PgPool, stores: &[StoreSpec<'_>], write: u16) -> anyhow::Result<Bench> {
    let serial = SERIAL.lock().await;
    let live = Live::from_env();
    for spec in stores.iter().filter(|spec| spec.endpoint.is_none()) {
        live.empty(spec.bucket).await?;
    }
    let secrets: Vec<(u16, &str)> = stores
        .iter()
        .map(|store| (store.id, live.secret.as_str()))
        .collect();
    let dir = keys_dir(&secrets);
    let ids: Vec<String> = stores.iter().map(|store| store.id.to_string()).collect();
    let list = ids.join(",");
    let media = MediaConfig::parse(
        |name| {
            if name == "CHAT_STORES" {
                return Some(list.clone());
            }
            if name == "CHAT_STORE_WRITE" {
                return Some(write.to_string());
            }
            stores.iter().find_map(|store| {
                let prefix = format!("CHAT_STORE_{}_", store.id);
                let field = name.strip_prefix(&prefix)?;
                let endpoint = store.endpoint.unwrap_or(&live.endpoint).to_owned();
                match field {
                    "ENDPOINT" | "PRESIGN" => Some(endpoint),
                    "BUCKET" => Some(store.bucket.to_owned()),
                    "ACCESS_KEY" => Some(live.access_key.clone()),
                    "CAP_GB" => Some("200".to_owned()),
                    _ => None,
                }
            })
        },
        &dir,
    );
    let redis = crate::redis::connect_url(&crate::modules::chat::sweeps::tests::support::redis_db(
        REDIS_DB,
    ))
    .expect("a redis pool");
    let chat = state_with(pg.clone(), redis, media);
    let mut conn = chat.redis.get().await?;
    let _: () = redis::cmd("DEL")
        .arg(reconcile::cursor::KEY)
        .arg(mover::cursor_key(1, 2))
        .query_async(&mut conn)
        .await?;
    Ok(Bench {
        chat,
        pg,
        _serial: serial,
    })
}

pub async fn bucket(index: usize) -> anyhow::Result<String> {
    Live::from_env().bucket(index).await
}

pub fn store(id: u16, bucket: &str) -> StoreSpec<'_> {
    StoreSpec {
        id,
        bucket,
        endpoint: None,
    }
}

pub fn ciphertext(length: i64) -> Vec<u8> {
    let mut bytes = vec![0; usize::try_from(length).unwrap_or(0)];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

pub async fn put_objects(chat: &ChatState, upload: &Upload) -> anyhow::Result<()> {
    let store = u16::try_from(upload.store)?;
    for (key, length) in upload.objects() {
        ok(chat.s3.put(store, &key, ciphertext(length)).await)?;
    }
    Ok(())
}

pub async fn exists(chat: &ChatState, store: u16, key: &str) -> anyhow::Result<bool> {
    Ok(ok(chat.s3.head(store, key).await)?.is_some())
}

pub async fn present(chat: &ChatState, upload: &Upload) -> anyhow::Result<Vec<bool>> {
    let store = u16::try_from(upload.store)?;
    let mut all = Vec::new();
    for (key, _) in upload.objects() {
        all.push(exists(chat, store, &key).await?);
    }
    Ok(all)
}

pub async fn listing(chat: &ChatState, store: u16) -> anyhow::Result<Vec<String>> {
    Ok(ok(chat.s3.list(store, None, 1_000).await)?
        .into_iter()
        .map(|listed| listed.key)
        .collect())
}

pub fn later_today() -> DateTime<Utc> {
    Utc::now()
        .date_naive()
        .and_hms_opt(23, 59, 0)
        .expect("a time")
        .and_utc()
}

pub fn days_later(days: i64) -> DateTime<Utc> {
    (Utc::now().date_naive() + Duration::days(days))
        .and_hms_opt(2, 0, 0)
        .expect("a time")
        .and_utc()
}
