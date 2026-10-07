use std::path::PathBuf;

use axum::extract::multipart::{Field, Multipart, MultipartError};
use axum::http::StatusCode;
use bytes::Bytes;
use catalog_ingest::TrackUpdate;
use sc_transport::{TrackUpload, UploadAsset, UploadImage};
use serde_json::{Map, Value, json};
use tokio::io::AsyncWriteExt;

use crate::error::{AppError, AppResult};

pub const ASSET_MAX_BYTES: u64 = 300 * 1024 * 1024;
const ARTWORK_MAX_BYTES: usize = 8 * 1024 * 1024;
const TEXT_MAX_BYTES: usize = 64 * 1024;
const FILE_NAME_MAX_CHARS: usize = 120;
pub const BODY_MAX_BYTES: usize = ASSET_MAX_BYTES as usize + ARTWORK_MAX_BYTES + 1024 * 1024;

const TEXT_FIELDS: &[&str] = &[
    "title",
    "description",
    "genre",
    "tag_list",
    "sharing",
    "release_date",
];
const AUDIO_EXTENSIONS: &[&str] = &[
    "aac", "aif", "aifc", "aiff", "alac", "amr", "flac", "m4a", "mp2", "mp3", "oga", "ogg", "wav",
    "wma",
];
const ARTWORK_TYPES: &[(&str, &str)] = &[("image/jpeg", "jpg"), ("image/png", "png")];

pub struct Spool {
    path: PathBuf,
}

impl Spool {
    fn create() -> Self {
        Self {
            path: std::env::temp_dir().join(format!("scd-upload-{}", uuid::Uuid::now_v7())),
        }
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct ParsedUpload {
    pub upload: TrackUpload,
    _spool: Spool,
}

pub async fn read(mut multipart: Multipart) -> AppResult<ParsedUpload> {
    let mut fields = Map::new();
    let mut artwork = None;
    let mut asset: Option<(Spool, UploadAsset)> = None;
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        let name = field.name().unwrap_or_default().to_owned();
        match name.as_str() {
            "asset" if asset.is_none() => asset = Some(spool_asset(field).await?),
            "artwork" if artwork.is_none() => artwork = Some(read_artwork(field).await?),
            key if TEXT_FIELDS.contains(&key) && !fields.contains_key(key) => {
                fields.insert(name, Value::String(read_text(field).await?));
            }
            _ => return Err(invalid("Unexpected or repeated upload field")),
        }
    }
    let (spool, asset) = asset.ok_or_else(|| invalid("The audio file is missing"))?;
    Ok(ParsedUpload {
        upload: TrackUpload {
            fields: track_fields(fields)?,
            artwork,
            asset,
        },
        _spool: spool,
    })
}

pub fn track_fields(fields: Map<String, Value>) -> AppResult<Vec<(String, String)>> {
    let title = fields.get("title").and_then(Value::as_str).unwrap_or("");
    if title.trim().is_empty() {
        return Err(invalid("The track needs a title"));
    }
    TrackUpdate::parse(&json!({ "track": fields })).map_err(invalid)?;
    Ok(fields
        .into_iter()
        .filter_map(|(key, value)| match value {
            Value::String(text) => Some((key, text.trim().to_owned())),
            _ => None,
        })
        .collect())
}

pub fn asset_file_name(raw: Option<&str>) -> AppResult<String> {
    let base = raw
        .unwrap_or_default()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim();
    let extension = base
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    if !AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        return Err(AppError::coded(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "upload_format_unsupported",
            "SoundCloud does not accept this audio format",
        ));
    }
    let stem: String = base[..base.len() - extension.len() - 1]
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '\\'))
        .take(FILE_NAME_MAX_CHARS)
        .collect();
    let stem = if stem.trim().is_empty() {
        "track".to_owned()
    } else {
        stem.trim().to_owned()
    };
    Ok(format!("{stem}.{extension}"))
}

async fn spool_asset(mut field: Field<'_>) -> AppResult<(Spool, UploadAsset)> {
    let file_name = asset_file_name(field.file_name())?;
    let spool = Spool::create();
    let mut file = tokio::fs::File::create(&spool.path)
        .await
        .map_err(|error| AppError::internal(format!("upload spool: {error}")))?;
    let mut len: u64 = 0;
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        len += chunk.len() as u64;
        if len > ASSET_MAX_BYTES {
            return Err(too_large());
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| AppError::internal(format!("upload spool: {error}")))?;
    }
    file.flush()
        .await
        .map_err(|error| AppError::internal(format!("upload spool: {error}")))?;
    if len == 0 {
        return Err(invalid("The audio file is empty"));
    }
    let asset = UploadAsset {
        path: spool.path.clone(),
        len,
        file_name,
    };
    Ok((spool, asset))
}

async fn read_artwork(field: Field<'_>) -> AppResult<UploadImage> {
    let mime = field
        .content_type()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = ARTWORK_TYPES
        .iter()
        .find(|(kind, _)| *kind == mime)
        .map(|(_, extension)| *extension)
        .ok_or_else(|| {
            AppError::coded(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "upload_artwork_unsupported",
                "Artwork must be a JPEG or PNG image",
            )
        })?;
    let bytes = read_capped(field, ARTWORK_MAX_BYTES).await?;
    Ok(UploadImage {
        bytes,
        file_name: format!("artwork.{extension}"),
        mime,
    })
}

async fn read_text(field: Field<'_>) -> AppResult<String> {
    let bytes = read_capped(field, TEXT_MAX_BYTES).await?;
    String::from_utf8(bytes.to_vec()).map_err(|_| invalid("Upload fields must be UTF-8 text"))
}

async fn read_capped(mut field: Field<'_>, max: usize) -> AppResult<Bytes> {
    let mut buffer = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        if buffer.len() + chunk.len() > max {
            return Err(too_large());
        }
        buffer.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(buffer))
}

fn multipart_error(error: MultipartError) -> AppError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return too_large();
    }
    invalid(error.body_text())
}

fn too_large() -> AppError {
    AppError::coded(
        StatusCode::PAYLOAD_TOO_LARGE,
        "upload_too_large",
        "The file is larger than the upload limit",
    )
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::coded(StatusCode::BAD_REQUEST, "upload_invalid", message)
}
