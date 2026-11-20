use std::time::Duration;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::primitives::ByteStream;

use crate::config::env_opt;

const REGION: &str = "us-east-1";
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(20);
const CONTENT_TYPE: &str = "image/jpeg";

#[derive(Clone)]
pub struct ArtworkStore {
    client: Client,
    bucket: String,
}

impl ArtworkStore {
    pub fn from_env() -> Option<Self> {
        let endpoint = env_opt("PLAYLIST_ARTWORK_S3_ENDPOINT")?;
        let (Some(bucket), Some(access_key), Some(secret_key)) = (
            env_opt("PLAYLIST_ARTWORK_S3_BUCKET"),
            env_opt("PLAYLIST_ARTWORK_S3_ACCESS_KEY_ID"),
            env_opt("PLAYLIST_ARTWORK_S3_SECRET_ACCESS_KEY"),
        ) else {
            panic!(
                "PLAYLIST_ARTWORK_S3_BUCKET, _ACCESS_KEY_ID and _SECRET_ACCESS_KEY are required \
                 when PLAYLIST_ARTWORK_S3_ENDPOINT is set"
            );
        };
        Some(Self::new(&endpoint, &bucket, &access_key, &secret_key))
    }

    pub fn new(endpoint: &str, bucket: &str, access_key: &str, secret_key: &str) -> Self {
        let credentials = Credentials::new(access_key, secret_key, None, None, "playlist-artwork");
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
        Self {
            client: Client::from_conf(config),
            bucket: bucket.to_owned(),
        }
    }

    pub async fn put(&self, key: &str, jpeg: Vec<u8>) -> Result<(), String> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(CONTENT_TYPE)
            .body(ByteStream::from(jpeg))
            .send()
            .await
            .map(|_| ())
            .map_err(|error| aws_sdk_s3::error::DisplayErrorContext(error).to_string())
    }
}
