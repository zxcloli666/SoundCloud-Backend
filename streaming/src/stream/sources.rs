use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use bytes::Bytes;
use futures::StreamExt;
use tracing::{info, warn};

use super::quality;
use super::restricted::RestrictedSource;
use crate::AppState;
use crate::error::AppError;

pub(super) type Fetched = (Bytes, &'static str, &'static str);

#[derive(Clone, Copy)]
pub(super) struct UploadRule {
    pub cacheable: bool,
    pub kept: Option<&'static str>,
}

impl UploadRule {
    pub(super) fn takes(self, quality: &str) -> bool {
        self.cacheable
            && self
                .kept
                .is_none_or(|kept| quality::rank(quality) > quality::rank(kept))
    }
}

async fn try_oauth(
    state: &AppState,
    access_token: &str,
    track_urn: &str,
    secret_token: Option<&str>,
    hq_only: bool,
) -> Option<Fetched> {
    let ctx = super::oauth::OauthCtx {
        client: &state.http_client,
        pg: &state.pg,
        proxy_url: &state.config.sc_proxy_url,
        proxy_fallback: state.config.sc_proxy_fallback,
        fallback_session_count: state.config.sc_oauth_fallback_sessions,
    };
    let result =
        super::oauth::try_oauth_stream(&ctx, access_token, track_urn, secret_token, hq_only)
            .await?;
    Some((result.data, result.content_type, result.quality))
}

pub(super) async fn try_session_oauth(
    state: &AppState,
    access_token: Option<&str>,
    track_urn: &str,
    secret_token: Option<&str>,
    hq_only: bool,
) -> Option<Fetched> {
    let access_token = access_token?;
    try_oauth(state, access_token, track_urn, secret_token, hq_only).await
}

pub(super) async fn try_cookies(
    state: &AppState,
    track_urn: &str,
    tag: &str,
    hq_only: bool,
) -> Option<Fetched> {
    let cookies_client = state.cookies.as_ref()?;
    match cookies_client.get_stream(track_urn, hq_only).await {
        Ok(Some(result)) => Some((result.data, result.content_type, result.quality)),
        Ok(None) => {
            warn!("{tag} {track_urn} cookies returned nothing");
            None
        }
        Err(e) => {
            warn!("{tag} {track_urn} cookies failed: {e}");
            None
        }
    }
}

pub(super) async fn try_anon(
    state: &AppState,
    track_urn: &str,
    tag: &str,
    at_least: &'static str,
) -> Option<Fetched> {
    match state.anon.get_stream(track_urn, at_least).await {
        Ok(Some(result)) => Some((result.data, result.content_type, result.quality)),
        Ok(None) => {
            info!("{tag} {track_urn} anon has nothing at {at_least} or above");
            None
        }
        Err(e) => {
            warn!("{tag} {track_urn} anon failed: {e}");
            None
        }
    }
}

async fn restricted_source(
    state: &AppState,
    track_urn: &str,
    tag: &str,
    hq_first: bool,
) -> Option<RestrictedSource> {
    match state.anon.resolve_restricted(track_urn, hq_first).await {
        Ok(Some(v)) => return Some(v),
        Ok(None) => {}
        Err(e) => warn!("{tag} {track_urn} restricted(anon) failed: {e}"),
    }
    let cookies = state.cookies.as_ref()?;
    match cookies.resolve_restricted(track_urn, hq_first).await {
        Ok(Some(v)) => Some(v),
        Ok(None) => None,
        Err(e) => {
            warn!("{tag} {track_urn} restricted(cookies) failed: {e}");
            None
        }
    }
}

pub(super) async fn try_relay_track(
    state: &AppState,
    track_urn: &str,
    quality: &'static str,
) -> Option<Fetched> {
    let id = track_urn.rsplit(':').next()?;
    if id.is_empty() || id == track_urn {
        return None;
    }
    let client_id = state.anon.get_client_id().await.unwrap_or_default();
    let (audio, ct) = crate::stream::proxy::get_track_via_relay(
        id,
        quality,
        &client_id,
        state.config.edge_wvd_url.as_deref(),
        state.config.edge_wvd_token.as_deref(),
    )
    .await?;
    Some((
        Bytes::from(audio),
        crate::stream::hls::mime_to_content_type(&ct),
        quality,
    ))
}

pub(super) async fn try_restricted(
    state: &AppState,
    track_urn: &str,
    tag: &str,
    hq_first: bool,
    at_least: &'static str,
    upload: UploadRule,
) -> Option<(Response, &'static str)> {
    let src = restricted_source(state, track_urn, tag, hq_first).await?;
    let quality = src.quality;
    if quality::rank(quality) < quality::rank(at_least) {
        info!("{tag} {track_urn} restricted has only {quality}, wanted {at_least}");
        return None;
    }

    if let (Some(wvd_url), Some(wvd_token)) = (
        state.config.edge_wvd_url.as_deref(),
        state.config.edge_wvd_token.as_deref(),
    ) && let Some(audio) =
        crate::stream::proxy::hls_decrypt_via_relay(&src.manifest, &src.token, wvd_url, wvd_token)
            .await
    {
        let bytes = Bytes::from(audio);
        if upload.takes(quality) && bytes.len() > 8192 {
            state.storage.upload_in_background_with_quality(
                track_urn.to_string(),
                bytes.clone(),
                quality,
            );
        }
        return Some((
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", src.content_type)
                .header("x-audio-quality", quality)
                .body(Body::from(bytes))
                .unwrap(),
            quality,
        ));
    }

    let engine = state.decryptor.as_ref()?;
    let fetcher: std::sync::Arc<dyn decrypt::Fetcher> =
        std::sync::Arc::new(crate::stream::decrypt_fetch::ProxyFetcher {
            client: state.http_client.clone(),
            proxy_url: state.config.sc_proxy_url.clone(),
        });
    let stream = match engine
        .process_stream(&src.manifest, &src.token, fetcher)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            warn!("{tag} {track_urn} restricted decode failed: {e}");
            return None;
        }
    };

    if !upload.takes(quality) {
        return Some((
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", src.content_type)
                .header("x-audio-quality", quality)
                .body(Body::from_stream(stream))
                .unwrap(),
            quality,
        ));
    }

    let acc = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let acc_w = acc.clone();
    let storage = state.storage.clone();
    let urn = track_urn.to_string();
    let teed = stream
        .map(move |chunk| {
            if let Ok(b) = &chunk {
                acc_w.lock().unwrap().extend_from_slice(b);
            }
            chunk
        })
        .chain(futures::stream::once(async move {
            let data = std::mem::take(&mut *acc.lock().unwrap());
            if data.len() > 8192 {
                storage.upload_in_background_with_quality(urn, Bytes::from(data), quality);
            }
            Ok::<_, decrypt::Error>(Bytes::new())
        }));

    Some((
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", src.content_type)
            .header("x-audio-quality", quality)
            .body(Body::from_stream(teed))
            .unwrap(),
        quality,
    ))
}

pub(super) fn respond_with_data(
    state: &AppState,
    track_urn: &str,
    (data, content_type, quality): Fetched,
    upload: UploadRule,
) -> Result<Response, AppError> {
    if upload.takes(quality) && data.len() > 8192 {
        state.storage.upload_in_background_with_quality(
            track_urn.to_string(),
            data.clone(),
            quality,
        );
    }

    data_response(data, content_type, quality)
}

pub(super) fn data_response(
    data: Bytes,
    content_type: &'static str,
    quality: &'static str,
) -> Result<Response, AppError> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", content_type)
        .header("content-length", data.len().to_string())
        .header("x-audio-quality", quality)
        .body(Body::from(data))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_only_gets_a_copy_better_than_the_one_it_keeps() {
        let empty = UploadRule {
            cacheable: true,
            kept: None,
        };
        assert!(empty.takes(quality::SQ));

        let kept_hq = UploadRule {
            cacheable: true,
            kept: Some(quality::HQ),
        };
        assert!(!kept_hq.takes(quality::SQ));
        assert!(!kept_hq.takes(quality::HQ));
        assert!(kept_hq.takes(quality::UQ));

        let private = UploadRule {
            cacheable: false,
            kept: None,
        };
        assert!(!private.takes(quality::UQ));
    }
}
