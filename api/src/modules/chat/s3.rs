use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use chrono::{DateTime, Utc};

use crate::modules::chat::errors::BusySource;
use crate::modules::chat::media::limits::PUT_CONTENT_TYPE;
use crate::modules::chat::media::wire::PresignedPut;
use crate::modules::chat::media::{MediaConfig, StoreConfig};
use crate::modules::chat::{ChatError, ChatResult};

const REGION: &str = "us-east-1";
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

struct StoreClients {
    ops: Client,
    signer: Client,
    bucket: String,
}

pub struct S3 {
    stores: HashMap<u16, StoreClients>,
}

fn client(endpoint: &str, store: &StoreConfig) -> Option<Client> {
    let secret = store.secret.as_ref()?.expose().clone();
    let credentials = Credentials::new(store.access_key.clone(), secret, None, None, "chat-store");
    let config = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(REGION))
        .endpoint_url(endpoint)
        .credentials_provider(credentials)
        .force_path_style(true)
        .timeout_config(
            TimeoutConfig::builder()
                .operation_attempt_timeout(ATTEMPT_TIMEOUT)
                .operation_timeout(OPERATION_TIMEOUT)
                .build(),
        )
        .build();
    Some(Client::from_conf(config))
}

impl S3 {
    pub fn new(config: &MediaConfig) -> Self {
        let stores = config
            .stores
            .iter()
            .filter_map(|store| {
                let clients = StoreClients {
                    ops: client(&store.endpoint, store)?,
                    signer: client(&store.presign, store)?,
                    bucket: store.bucket.clone(),
                };
                Some((store.id, clients))
            })
            .collect();
        Self { stores }
    }

    pub fn serves(&self, id: u16) -> bool {
        self.stores.contains_key(&id)
    }

    fn store(&self, id: u16) -> ChatResult<&StoreClients> {
        self.stores.get(&id).ok_or(ChatError::Internal)
    }

    pub async fn presign_put(
        &self,
        store: u16,
        key: &str,
        length: u64,
        ttl: Duration,
    ) -> ChatResult<PresignedPut> {
        let clients = self.store(store)?;
        let signed = clients
            .signer
            .put_object()
            .bucket(&clients.bucket)
            .key(key)
            .content_length(i64::try_from(length).map_err(|_| ChatError::Invalid)?)
            .content_type(PUT_CONTENT_TYPE)
            .presigned(presigning(ttl)?)
            .await
            .map_err(|error| {
                tracing::error!(%error, "a chat PUT was not presigned");
                ChatError::Internal
            })?;
        let headers = BTreeMap::from([
            ("content-length".to_owned(), length.to_string()),
            ("content-type".to_owned(), PUT_CONTENT_TYPE.to_owned()),
        ]);
        Ok(PresignedPut {
            url: signed.uri().to_owned(),
            headers,
        })
    }

    pub async fn presign_get(&self, store: u16, key: &str, ttl: Duration) -> ChatResult<String> {
        let clients = self.store(store)?;
        let signed = clients
            .signer
            .get_object()
            .bucket(&clients.bucket)
            .key(key)
            .presigned(presigning(ttl)?)
            .await
            .map_err(|error| {
                tracing::error!(%error, "a chat GET was not presigned");
                ChatError::Internal
            })?;
        Ok(signed.uri().to_owned())
    }

    pub async fn head(&self, store: u16, key: &str) -> ChatResult<Option<u64>> {
        let clients = self.store(store)?;
        match clients
            .ops
            .head_object()
            .bucket(&clients.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(head) => Ok(Some(
                head.content_length()
                    .and_then(|length| u64::try_from(length).ok())
                    .unwrap_or(0),
            )),
            Err(error) if missing(error.raw_response().map(|raw| raw.status().as_u16())) => {
                Ok(None)
            }
            Err(error) => Err(unavailable("HEAD", &error)),
        }
    }

    pub async fn get(&self, store: u16, key: &str) -> ChatResult<Option<Vec<u8>>> {
        let clients = self.store(store)?;
        let object = match clients
            .ops
            .get_object()
            .bucket(&clients.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(object) => object,
            Err(error) if missing(error.raw_response().map(|raw| raw.status().as_u16())) => {
                return Ok(None);
            }
            Err(error) => return Err(unavailable("GET", &error)),
        };
        let body = object
            .body
            .collect()
            .await
            .map_err(|error| unavailable("GET body", &error))?;
        Ok(Some(body.into_bytes().to_vec()))
    }

    pub async fn put(&self, store: u16, key: &str, bytes: Vec<u8>) -> ChatResult<()> {
        let clients = self.store(store)?;
        clients
            .ops
            .put_object()
            .bucket(&clients.bucket)
            .key(key)
            .content_type(PUT_CONTENT_TYPE)
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(|error| unavailable("PUT", &error))?;
        Ok(())
    }

    pub async fn copy(&self, store: u16, from: &str, to: &str) -> ChatResult<bool> {
        let clients = self.store(store)?;
        match clients
            .ops
            .copy_object()
            .bucket(&clients.bucket)
            .key(to)
            .copy_source(format!("{}/{from}", clients.bucket))
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(error) if missing(error.raw_response().map(|raw| raw.status().as_u16())) => {
                Ok(false)
            }
            Err(error) => Err(unavailable("COPY", &error)),
        }
    }

    pub async fn delete(&self, store: u16, key: &str) -> ChatResult<()> {
        let clients = self.store(store)?;
        match clients
            .ops
            .delete_object()
            .bucket(&clients.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if missing(error.raw_response().map(|raw| raw.status().as_u16())) => Ok(()),
            Err(error) => Err(unavailable("DELETE", &error)),
        }
    }

    pub async fn list(
        &self,
        store: u16,
        after: Option<&str>,
        page: usize,
    ) -> ChatResult<Vec<Listed>> {
        let clients = self.store(store)?;
        let listed = clients
            .ops
            .list_objects_v2()
            .bucket(&clients.bucket)
            .set_start_after(after.map(str::to_owned))
            .max_keys(i32::try_from(page).unwrap_or(1000))
            .send()
            .await
            .map_err(|error| unavailable("LIST", &error))?;
        Ok(listed
            .contents()
            .iter()
            .filter_map(|object| {
                let key = object.key()?.to_owned();
                let modified = object
                    .last_modified()
                    .and_then(|at| DateTime::<Utc>::from_timestamp(at.secs(), at.subsec_nanos()))
                    .unwrap_or_else(Utc::now);
                Some(Listed { key, modified })
            })
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub key: String,
    pub modified: DateTime<Utc>,
}

fn missing(status: Option<u16>) -> bool {
    status == Some(404)
}

fn unavailable(operation: &str, error: &dyn std::fmt::Display) -> ChatError {
    tracing::warn!(operation, %error, "a chat store call failed");
    ChatError::busy(BusySource::S3)
}

fn presigning(ttl: Duration) -> ChatResult<PresigningConfig> {
    PresigningConfig::expires_in(ttl).map_err(|_| ChatError::Internal)
}

#[cfg(test)]
#[path = "tests/s3_tests.rs"]
mod tests;
