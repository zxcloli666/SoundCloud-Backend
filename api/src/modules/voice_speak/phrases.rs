use std::sync::Arc;
use std::time::Duration;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use futures::future::BoxFuture;

use super::stores::{Objects, StoreError};
use crate::config::PhrasesCfg;

const REGION: &str = "us-east-1";
const READ_BUDGET: Duration = Duration::from_secs(3);
const WRITE_BUDGET: Duration = Duration::from_secs(10);
const CLIP_TYPE: &str = "audio/ogg";

pub struct S3Phrases {
    client: Client,
    bucket: String,
}

impl S3Phrases {
    pub fn connect(config: &PhrasesCfg) -> Result<Arc<Self>, String> {
        let keys = config.keys()?;
        let credentials = Credentials::new(
            keys.access_key.expose().clone(),
            keys.secret_key.expose().clone(),
            None,
            None,
            "speak-phrases",
        );
        let s3 = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(REGION))
            .endpoint_url(config.endpoint.trim())
            .credentials_provider(credentials)
            .force_path_style(true)
            .build();
        Ok(Arc::new(Self {
            client: Client::from_conf(s3),
            bucket: config.bucket.trim().to_owned(),
        }))
    }

    async fn fetch(&self, key: &str) -> Result<Option<Bytes>, StoreError> {
        let sent = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await;
        let object = match sent {
            Ok(object) => object,
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(GetObjectError::is_no_such_key)
                    || error
                        .raw_response()
                        .is_some_and(|raw| raw.status().as_u16() == 404) =>
            {
                return Ok(None);
            }
            Err(_) => return Err(StoreError::Unavailable),
        };
        let body = object
            .body
            .collect()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        Ok(Some(body.into_bytes()))
    }

    async fn store(&self, key: &str, clip: Bytes) -> Result<(), StoreError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(CLIP_TYPE)
            .body(ByteStream::from(clip))
            .send()
            .await
            .map(|_| ())
            .map_err(|_| StoreError::Unavailable)
    }
}

impl Objects for S3Phrases {
    fn read<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<Bytes>, StoreError>> {
        Box::pin(async move {
            tokio::time::timeout(READ_BUDGET, self.fetch(name))
                .await
                .map_err(|_| StoreError::TimedOut)?
        })
    }

    fn write<'a>(&'a self, name: &'a str, bytes: Bytes) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            tokio::time::timeout(WRITE_BUDGET, self.store(name, bytes))
                .await
                .map_err(|_| StoreError::TimedOut)?
        })
    }
}
