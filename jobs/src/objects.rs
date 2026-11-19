use std::future::Future;
use std::path::Path;
use std::time::{Duration, SystemTime};

use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::primitives::ByteStream;

use crate::config::ObjectsConfig;

const LIST_PAGE: i32 = 1000;

#[derive(Debug, thiserror::Error)]
pub enum ObjectsError {
    #[error("object store {action} {key} failed: {source}")]
    Failed {
        action: &'static str,
        key: String,
        source: anyhow::Error,
    },
}

#[derive(Clone)]
pub struct Objects {
    client: Client,
    bucket: String,
}

pub struct Listed {
    pub key: String,
    pub modified: SystemTime,
}

impl Objects {
    pub async fn connect(config: &ObjectsConfig) -> Self {
        let credentials = Credentials::new(
            config.access_key_id.expose(),
            config.secret_access_key.expose(),
            None,
            None,
            "jobs-env",
        );
        let shared = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(config.region.clone()))
            .credentials_provider(credentials)
            .endpoint_url(config.endpoint.as_str())
            .load()
            .await;
        let client = Client::from_conf(
            aws_sdk_s3::config::Builder::from(&shared)
                .force_path_style(config.path_style)
                .build(),
        );
        Self {
            client,
            bucket: config.bucket.clone(),
        }
    }
}

pub trait Blobs: Send + Sync {
    fn put_file(
        &self,
        key: &str,
        path: &Path,
    ) -> impl Future<Output = Result<(), ObjectsError>> + Send;
    fn put_bytes(
        &self,
        key: &str,
        bytes: Vec<u8>,
    ) -> impl Future<Output = Result<(), ObjectsError>> + Send;
    fn copy(&self, from: &str, to: &str) -> impl Future<Output = Result<(), ObjectsError>> + Send;
    fn delete(&self, key: &str) -> impl Future<Output = Result<(), ObjectsError>> + Send;
    fn list(&self, prefix: &str) -> impl Future<Output = Result<Vec<Listed>, ObjectsError>> + Send;
}

impl Objects {
    pub async fn get_bytes(&self, key: &str) -> Result<Option<Vec<u8>>, ObjectsError> {
        let answer = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await;
        let object = match answer {
            Ok(object) => object,
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|error| error.is_no_such_key()) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(failed("get", key, error)),
        };
        let bytes = object
            .body
            .collect()
            .await
            .map_err(|error| failed("read", key, error))?;
        Ok(Some(bytes.into_bytes().to_vec()))
    }
}

impl Blobs for Objects {
    async fn put_file(&self, key: &str, path: &Path) -> Result<(), ObjectsError> {
        let body = ByteStream::from_path(path)
            .await
            .map_err(|error| failed("read", key, error))?;
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type("video/mp4")
            .body(body)
            .send()
            .await
            .map_err(|error| failed("put", key, error))?;
        Ok(())
    }

    async fn put_bytes(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectsError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type("video/mp4")
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(|error| failed("put", key, error))?;
        Ok(())
    }

    async fn copy(&self, from: &str, to: &str) -> Result<(), ObjectsError> {
        self.client
            .copy_object()
            .bucket(&self.bucket)
            .copy_source(format!("{}/{from}", self.bucket))
            .key(to)
            .send()
            .await
            .map_err(|error| failed("copy", to, error))?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectsError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|error| failed("delete", key, error))?;
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<Listed>, ObjectsError> {
        let mut listed = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix)
                .max_keys(LIST_PAGE)
                .set_continuation_token(token.take())
                .send()
                .await
                .map_err(|error| failed("list", prefix, error))?;
            listed.extend(page.contents().iter().filter_map(|object| {
                let key = object.key()?.to_owned();
                let seconds = object.last_modified()?.secs();
                let modified = SystemTime::UNIX_EPOCH
                    + Duration::from_secs(u64::try_from(seconds).unwrap_or(0));
                Some(Listed { key, modified })
            }));
            match page.next_continuation_token() {
                Some(next) if page.is_truncated().unwrap_or(false) => token = Some(next.to_owned()),
                _ => return Ok(listed),
            }
        }
    }
}

fn failed(
    action: &'static str,
    key: &str,
    source: impl std::error::Error + Send + Sync + 'static,
) -> ObjectsError {
    ObjectsError::Failed {
        action,
        key: key.to_owned(),
        source: source.into(),
    }
}
