use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use serde_json::Value;
use tokio_util::io::ReaderStream;
use wreq::header::{HeaderName, HeaderValue, RETRY_AFTER};
use wreq::multipart::{Form, Part};
use wreq::{Body, Method};

use crate::client::{
    RESPONSE_MAX_BYTES, ScClient, api_error, auth_headers, collect_capped, decode_json,
    parse_retry_after,
};
use crate::error::{ScError, ScResult};

const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct UploadAsset {
    pub path: PathBuf,
    pub len: u64,
    pub file_name: String,
}

pub struct UploadImage {
    pub bytes: Bytes,
    pub file_name: String,
    pub mime: String,
}

pub struct TrackUpload {
    pub fields: Vec<(String, String)>,
    pub artwork: Option<UploadImage>,
    pub asset: UploadAsset,
}

enum Attempt {
    Done(ScResult<Value>),
    Unsent(ScError),
}

impl TrackUpload {
    async fn form(&self) -> ScResult<Form> {
        let mut form = Form::new();
        for (key, value) in &self.fields {
            form = form.text(format!("track[{key}]"), value.clone());
        }
        if let Some(artwork) = &self.artwork {
            let part = Part::bytes(artwork.bytes.to_vec())
                .file_name(artwork.file_name.clone())
                .mime_str(&artwork.mime)
                .map_err(|error| ScError::invalid(format!("artwork type: {error}")))?;
            form = form.part("track[artwork_data]", part);
        }
        let file = tokio::fs::File::open(&self.asset.path)
            .await
            .map_err(|error| ScError::invalid(format!("upload spool: {error}")))?;
        let body = Body::wrap_stream(ReaderStream::new(file));
        let part = Part::stream_with_length(body, self.asset.len)
            .file_name(self.asset.file_name.clone());
        Ok(form.part("track[asset_data]", part))
    }
}

impl ScClient {
    pub async fn upload_track(&self, access_token: &str, upload: &TrackUpload) -> ScResult<Value> {
        let target = format!("{}/tracks", self.api_base());
        let proxy = self.upload_proxy();
        let mut routes = Vec::with_capacity(2);
        if proxy.is_none() || self.proxy_fallback() {
            routes.push(None);
        }
        routes.extend(proxy.map(Some));
        let mut last = ScError::invalid("no upload route");
        for route in routes {
            match self.upload_once(&target, route, access_token, upload).await {
                Attempt::Done(result) => return result,
                Attempt::Unsent(error) => last = error,
            }
        }
        Err(last)
    }

    async fn upload_once(
        &self,
        target: &str,
        proxy: Option<&str>,
        access_token: &str,
        upload: &TrackUpload,
    ) -> Attempt {
        let form = match upload.form().await {
            Ok(form) => form,
            Err(error) => return Attempt::Done(Err(error)),
        };
        let mut request = self
            .http()
            .request(Method::POST, proxy.unwrap_or(target))
            .headers(auth_headers(access_token, false))
            .timeout(UPLOAD_TIMEOUT)
            .multipart(form);
        if proxy.is_some() {
            let encoded = base64::engine::general_purpose::STANDARD.encode(target);
            match HeaderValue::from_str(&encoded) {
                Ok(value) => request = request.header(HeaderName::from_static("x-target"), value),
                Err(error) => {
                    return Attempt::Done(Err(ScError::invalid(format!("bad x-target: {error}"))));
                }
            }
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) if error.is_connect() => {
                return Attempt::Unsent(ScError::unreachable(error.without_url().to_string()));
            }
            Err(error) => {
                return Attempt::Done(Err(ScError::unreachable(
                    error.without_url().to_string(),
                )));
            }
        };
        let status = response.status();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after);
        Attempt::Done(
            collect_capped(response, RESPONSE_MAX_BYTES)
                .await
                .and_then(|bytes| {
                    if status.is_client_error() || status.is_server_error() {
                        Err(api_error(status.as_u16(), &bytes, retry_after))
                    } else {
                        decode_json(&bytes)
                    }
                }),
        )
    }
}
