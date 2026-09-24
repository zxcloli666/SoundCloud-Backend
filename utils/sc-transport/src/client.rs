use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use bytes::Bytes;
use call_relay::{Client as RelayClient, Request as RelayRequest};
use futures::StreamExt;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use wreq::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, RETRY_AFTER,
    USER_AGENT,
};
use wreq::{Client, Method};

use crate::ScConfig;
use crate::error::{ScError, ScResult};
use crate::types::ScTokenResponse;

type RelayFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, call_relay::Error>> + Send + 'a>>;

pub trait RelayTransport: Send + Sync {
    fn fetch<'a>(&'a self, request: &'a RelayRequest) -> RelayFuture<'a, call_relay::Response>;

    fn call_method_rotated<'a>(
        &'a self,
        method_id: &'a str,
        script: &'a str,
        inputs: Bytes,
        region_rotation: i32,
    ) -> RelayFuture<'a, Bytes>;
}

impl RelayTransport for RelayClient {
    fn fetch<'a>(&'a self, request: &'a RelayRequest) -> RelayFuture<'a, call_relay::Response> {
        Box::pin(RelayClient::fetch(self, request))
    }

    fn call_method_rotated<'a>(
        &'a self,
        method_id: &'a str,
        script: &'a str,
        inputs: Bytes,
        region_rotation: i32,
    ) -> RelayFuture<'a, Bytes> {
        Box::pin(RelayClient::call_method_rotated(
            self,
            method_id,
            script,
            inputs,
            region_rotation,
        ))
    }
}

const API_BASE: &str = "https://api.soundcloud.com";
const AUTH_BASE: &str = "https://secure.soundcloud.com";
const SC_HOME: &str = "https://soundcloud.com";
const SC_WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                         (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const ANON_CID_TTL: Duration = Duration::from_secs(1800);
const ANON_CID_RETRY_BASE: Duration = Duration::from_secs(5);
const ANON_CID_RETRY_MAX: Duration = Duration::from_secs(300);
const TOKEN_RESPONSE_MAX_BYTES: usize = 64 * 1024;
pub const RESPONSE_MAX_BYTES: usize = 8 * 1024 * 1024;

static HYDRATION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#""hydratable"\s*:\s*"apiClient"\s*,\s*"data"\s*:\s*\{\s*"id"\s*:\s*"([^"]+)""#)
        .expect("hydration regex")
});

#[derive(Clone)]
pub struct OAuthCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

#[derive(Clone)]
pub struct ScClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: Client,
    api_base: String,
    home_base: String,
    proxy_url: String,
    proxy_fallback: bool,
    relay: Option<Arc<dyn RelayTransport>>,
    anon_client_id: RwLock<AnonClientId>,
    anon_refresh: Mutex<()>,
}

#[derive(Default)]
struct AnonClientId {
    value: Option<(String, Instant)>,
    failures: u32,
    retry_at: Option<Instant>,
}

impl AnonClientId {
    fn fresh(&self) -> Option<String> {
        self.value
            .as_ref()
            .filter(|(_, fetched_at)| fetched_at.elapsed() < ANON_CID_TTL)
            .map(|(id, _)| id.clone())
    }

    fn accept(&mut self, id: String) {
        self.value = Some((id, Instant::now()));
        self.failures = 0;
        self.retry_at = None;
    }

    fn reject(&mut self) {
        self.failures = self.failures.saturating_add(1);
        let exponent = self.failures.saturating_sub(1).min(6);
        let wait = ANON_CID_RETRY_BASE
            .saturating_mul(1_u32 << exponent)
            .min(ANON_CID_RETRY_MAX);
        self.retry_at = Some(Instant::now() + wait);
    }

    fn is_backing_off(&self) -> bool {
        self.retry_at
            .is_some_and(|retry_at| Instant::now() < retry_at)
    }
}

#[derive(Clone, Copy)]
enum Channel {
    Direct,
    Proxy,
    Relay,
}

impl ScClient {
    pub fn new(cfg: &ScConfig) -> Result<Self, wreq::Error> {
        let http = sc_fingerprint::builder(None)
            .tcp_keepalive(Duration::from_secs(60))
            .pool_max_idle_per_host(20)
            .pool_idle_timeout(Duration::from_secs(90))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self {
            inner: Arc::new(Inner {
                http,
                api_base: cfg.api_base.clone().unwrap_or_else(|| API_BASE.to_owned()),
                home_base: cfg.home_base.clone().unwrap_or_else(|| SC_HOME.to_owned()),
                proxy_url: cfg.proxy_url.clone(),
                proxy_fallback: cfg.proxy_fallback,
                relay: None,
                anon_client_id: RwLock::new(AnonClientId::default()),
                anon_refresh: Mutex::new(()),
            }),
        })
    }

    pub fn with_relay(self, relay: Arc<dyn RelayTransport>) -> Self {
        let inner = Arc::new(Inner {
            http: self.inner.http.clone(),
            api_base: self.inner.api_base.clone(),
            home_base: self.inner.home_base.clone(),
            proxy_url: self.inner.proxy_url.clone(),
            proxy_fallback: self.inner.proxy_fallback,
            relay: Some(relay),
            anon_client_id: RwLock::new(AnonClientId::default()),
            anon_refresh: Mutex::new(()),
        });
        let client = Self { inner };
        let warm = client.clone();
        tokio::spawn(async move {
            let _ = warm.anon_client_id().await;
        });
        client
    }

    pub fn auth_base_url(&self) -> &str {
        AUTH_BASE
    }

    pub fn has_relay(&self) -> bool {
        self.inner.relay.is_some()
    }

    pub async fn exchange_code_for_token(
        &self,
        code: &str,
        code_verifier: &str,
        creds: &OAuthCredentials,
    ) -> ScResult<ScTokenResponse> {
        let body = serde_urlencoded::to_string([
            ("grant_type", "authorization_code"),
            ("client_id", creds.client_id.as_str()),
            ("client_secret", creds.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", creds.redirect_uri.as_str()),
            ("code_verifier", code_verifier),
        ])
        .map_err(|e| ScError::invalid(format!("urlencode: {e}")))?;

        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );

        let url = format!("{AUTH_BASE}/oauth/token");
        let bytes = self
            .send_direct_limited(
                Method::POST,
                &url,
                headers,
                Some(Bytes::from(body)),
                TOKEN_RESPONSE_MAX_BYTES,
            )
            .await?;
        decode_json(&bytes)
    }

    pub async fn refresh_access_token(
        &self,
        refresh_token: &str,
        creds: &OAuthCredentials,
    ) -> ScResult<ScTokenResponse> {
        let body = serde_urlencoded::to_string([
            ("grant_type", "refresh_token"),
            ("client_id", creds.client_id.as_str()),
            ("client_secret", creds.client_secret.as_str()),
            ("refresh_token", refresh_token),
        ])
        .map_err(|e| ScError::invalid(format!("urlencode: {e}")))?;

        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );

        let url = format!("{AUTH_BASE}/oauth/token");
        let bytes = self
            .send_direct_limited(
                Method::POST,
                &url,
                headers,
                Some(Bytes::from(body)),
                TOKEN_RESPONSE_MAX_BYTES,
            )
            .await?;
        decode_json(&bytes)
    }

    pub async fn api_get<T: DeserializeOwned>(
        &self,
        path: &str,
        access_token: &str,
        params: Option<&[(String, String)]>,
    ) -> ScResult<T> {
        let url = build_api_url(&self.inner.api_base, path, params);
        let headers = auth_headers(access_token, false);
        let bytes = self.with_fallback(Method::GET, &url, headers, None).await?;
        decode_json(&bytes)
    }

    pub async fn api_get_value(
        &self,
        path: &str,
        access_token: &str,
        params: Option<&[(String, String)]>,
    ) -> ScResult<Value> {
        self.api_get::<Value>(path, access_token, params).await
    }

    pub async fn api_get_absolute_value(
        &self,
        absolute_url: &str,
        access_token: &str,
    ) -> ScResult<Value> {
        let headers = auth_headers(access_token, false);
        let bytes = self
            .with_fallback(Method::GET, absolute_url, headers, None)
            .await?;
        decode_json(&bytes)
    }

    pub async fn anon_get_via_relay_proxy(
        &self,
        target_url: &str,
        headers: HeaderMap,
    ) -> ScResult<Bytes> {
        self.race_relay_proxy(Method::GET, target_url, headers, None)
            .await
    }

    pub async fn resolve_track_via_relay(&self, url: &str) -> crate::RelayRead<Value> {
        let Ok(inputs) = serde_json::to_vec(&serde_json::json!({ "url": url })) else {
            return crate::RelayRead::Unavailable;
        };
        let answer = self
            .call_relay_method(
                "sc.resolve_track",
                crate::lua_methods::RESOLVE_TRACK,
                inputs,
            )
            .await;
        relay_entity(answer, "track")
    }

    pub async fn user_by_id_via_relay(&self, user_id: &str) -> crate::RelayRead<Value> {
        let Ok(inputs) = serde_json::to_vec(&serde_json::json!({ "id": user_id })) else {
            return crate::RelayRead::Unavailable;
        };
        let answer = self
            .call_relay_method("sc.user_by_id", crate::lua_methods::USER_BY_ID, inputs)
            .await;
        relay_entity(answer, "user")
    }

    pub async fn track_by_id_via_relay(&self, sc_track_id: &str) -> crate::RelayRead<Value> {
        let Ok(inputs) = serde_json::to_vec(&serde_json::json!({ "id": sc_track_id })) else {
            return crate::RelayRead::Unavailable;
        };
        let answer = self
            .call_relay_method("sc.track_by_id", crate::lua_methods::TRACK_BY_ID, inputs)
            .await;
        relay_entity(answer, "track")
    }

    pub async fn playlist_full_via_relay(
        &self,
        playlist_id: &str,
        hydrate: bool,
    ) -> crate::RelayRead<Value> {
        let Ok(inputs) =
            serde_json::to_vec(&serde_json::json!({ "id": playlist_id, "hydrate": hydrate }))
        else {
            return crate::RelayRead::Unavailable;
        };
        let answer = self
            .call_relay_method(
                "sc.playlist_full",
                crate::lua_methods::PLAYLIST_FULL,
                inputs,
            )
            .await;
        relay_entity(answer, "playlist")
    }

    pub async fn user_collection_via_relay(
        &self,
        user_id: &str,
        kind: &str,
        cursor: Option<&str>,
        limit: i64,
    ) -> Option<Value> {
        let inputs = serde_json::to_vec(&serde_json::json!({
            "user_id": user_id, "kind": kind, "cursor": cursor, "limit": limit,
        }))
        .ok()?;
        let v = self
            .call_relay_method(
                "sc.user_collection",
                crate::lua_methods::USER_COLLECTION,
                inputs,
            )
            .await?;
        (v.get("ok").and_then(Value::as_bool) == Some(true)).then_some(v)
    }

    pub async fn search_via_relay(
        &self,
        search_type: &str,
        q: &str,
        cursor: Option<&str>,
        limit: i64,
    ) -> Option<Value> {
        let inputs = serde_json::to_vec(&serde_json::json!({
            "type": search_type, "q": q, "cursor": cursor, "limit": limit,
        }))
        .ok()?;
        let v = self
            .call_relay_method("sc.search", crate::lua_methods::SEARCH, inputs)
            .await?;
        (v.get("ok").and_then(Value::as_bool) == Some(true)).then_some(v)
    }

    pub async fn apiv2_get_via_relay_rotated(
        &self,
        url: &str,
        region_rotation: i32,
    ) -> Option<Value> {
        let inputs = serde_json::to_vec(&serde_json::json!({ "url": url })).ok()?;
        let v = self
            .call_relay_method_rotated(
                "sc.apiv2_get",
                crate::lua_methods::APIV2_GET,
                inputs,
                region_rotation,
            )
            .await?;
        (v.get("ok").and_then(Value::as_bool) == Some(true))
            .then(|| v.get("data").cloned())
            .flatten()
    }

    async fn call_relay_method(
        &self,
        method_id: &'static str,
        script: &'static str,
        inputs: Vec<u8>,
    ) -> Option<Value> {
        self.call_relay_method_rotated(method_id, script, inputs, 0)
            .await
    }

    async fn call_relay_method_rotated(
        &self,
        method_id: &'static str,
        script: &'static str,
        inputs: Vec<u8>,
        region_rotation: i32,
    ) -> Option<Value> {
        let relay = self.inner.relay.as_ref()?;
        let inputs = self.inject_client_id(inputs).await;
        let out = match relay
            .call_method_rotated(method_id, script, Bytes::from(inputs), region_rotation)
            .await
        {
            Ok(b) => b,
            Err(e) => {
                if !e.is_disabled() {
                    tracing::debug!(error = %e, method = method_id, "relay lua method failed");
                }
                return None;
            }
        };
        serde_json::from_slice(&out).ok()
    }

    async fn inject_client_id(&self, inputs: Vec<u8>) -> Vec<u8> {
        let Some(cid) = self.anon_client_id().await else {
            return inputs;
        };
        match serde_json::from_slice::<Value>(&inputs) {
            Ok(mut v) if v.is_object() => {
                v["client_id"] = Value::String(cid);
                serde_json::to_vec(&v).unwrap_or(inputs)
            }
            _ => inputs,
        }
    }

    pub async fn anon_client_id(&self) -> Option<String> {
        if let Some(id) = self.inner.anon_client_id.read().await.fresh() {
            return Some(id);
        }
        self.refresh_anon_client_id(None).await
    }

    pub async fn rotate_anon_client_id(&self, stale: Option<&str>) -> Option<String> {
        self.refresh_anon_client_id(stale).await
    }

    async fn refresh_anon_client_id(&self, stale: Option<&str>) -> Option<String> {
        let _gate = self.inner.anon_refresh.lock().await;
        {
            let state = self.inner.anon_client_id.read().await;
            if let Some(id) = state.fresh()
                && stale != Some(id.as_str())
            {
                return Some(id);
            }
            if state.is_backing_off() {
                return None;
            }
        }
        let mut h = HeaderMap::new();
        h.insert(USER_AGENT, HeaderValue::from_static(SC_WEB_UA));
        let fetched = self
            .anon_get_via_relay_proxy(&self.inner.home_base, h)
            .await
            .ok()
            .and_then(|bytes| extract_anon_client_id(&String::from_utf8_lossy(&bytes)));
        let mut state = self.inner.anon_client_id.write().await;
        match fetched {
            Some(id) => {
                state.accept(id.clone());
                Some(id)
            }
            None => {
                state.reject();
                None
            }
        }
    }

    pub async fn api_put<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        access_token: &str,
        body: Option<&B>,
    ) -> ScResult<T> {
        let url = format!("{}{path}", self.inner.api_base);
        let headers = auth_headers(access_token, true);
        let payload = match body {
            Some(b) => Bytes::from(
                serde_json::to_vec(b).map_err(|e| ScError::invalid(format!("json encode: {e}")))?,
            ),
            None => Bytes::new(),
        };
        let bytes = self
            .with_fallback(Method::PUT, &url, headers, Some(payload))
            .await?;
        decode_json(&bytes)
    }

    pub async fn api_put_value(
        &self,
        path: &str,
        access_token: &str,
        body: Option<&Value>,
    ) -> ScResult<Value> {
        self.api_put::<Value, Value>(path, access_token, body).await
    }

    pub async fn api_delete(&self, path: &str, access_token: &str) -> ScResult<Value> {
        let url = format!("{}{path}", self.inner.api_base);
        let headers = auth_headers(access_token, false);
        let bytes = self
            .with_fallback(Method::DELETE, &url, headers, None)
            .await?;
        decode_json(&bytes)
    }

    async fn with_fallback(
        &self,
        method: Method,
        target_url: &str,
        headers: HeaderMap,
        body: Option<Bytes>,
    ) -> ScResult<Bytes> {
        let proxy_set = !self.inner.proxy_url.is_empty();
        let relay_set = self.inner.relay.is_some();
        let is_get = method == Method::GET;
        let pf = self.inner.proxy_fallback;

        if !proxy_set && !relay_set {
            return self.send_direct(method, target_url, headers, body).await;
        }

        if pf {
            if is_get {
                match self
                    .send_direct(method.clone(), target_url, headers.clone(), body.clone())
                    .await
                {
                    Ok(b) => return Ok(b),
                    Err(e) => tracing::debug!(error = %e, "direct failed, racing relay&proxy"),
                }
                self.race_relay_proxy(method, target_url, headers, body)
                    .await
            } else {
                self.try_chain(
                    method,
                    target_url,
                    headers,
                    body,
                    &[Channel::Direct, Channel::Proxy, Channel::Relay],
                )
                .await
            }
        } else if is_get {
            self.race_relay_proxy(method, target_url, headers, body)
                .await
        } else {
            self.try_chain(
                method,
                target_url,
                headers,
                body,
                &[Channel::Proxy, Channel::Relay],
            )
            .await
        }
    }

    async fn try_chain(
        &self,
        method: Method,
        target_url: &str,
        headers: HeaderMap,
        body: Option<Bytes>,
        chain: &[Channel],
    ) -> ScResult<Bytes> {
        let mut last: Option<ScError> = None;
        for ch in chain {
            let r = match ch {
                Channel::Direct => {
                    self.send_direct(method.clone(), target_url, headers.clone(), body.clone())
                        .await
                }
                Channel::Proxy => {
                    if self.inner.proxy_url.is_empty() {
                        continue;
                    }
                    self.send_proxy(method.clone(), target_url, headers.clone(), body.clone())
                        .await
                }
                Channel::Relay => {
                    if self.inner.relay.is_none() {
                        continue;
                    }
                    self.send_relay(
                        method.clone(),
                        target_url.to_string(),
                        headers.clone(),
                        body.clone(),
                    )
                    .await
                }
            };
            match r {
                Ok(b) => return Ok(b),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| ScError::invalid("no channels available")))
    }

    async fn race_relay_proxy(
        &self,
        method: Method,
        target_url: &str,
        headers: HeaderMap,
        body: Option<Bytes>,
    ) -> ScResult<Bytes> {
        let proxy_set = !self.inner.proxy_url.is_empty();
        let relay_set = self.inner.relay.is_some();
        match (relay_set, proxy_set) {
            (true, true) => {
                let m1 = method.clone();
                let u1 = target_url.to_string();
                let h1 = headers.clone();
                let b1 = body.clone();
                let relay_fut: std::pin::Pin<
                    Box<dyn std::future::Future<Output = ScResult<Bytes>> + Send + '_>,
                > = Box::pin(self.send_relay(m1, u1, h1, b1));
                let proxy_fut: std::pin::Pin<
                    Box<dyn std::future::Future<Output = ScResult<Bytes>> + Send + '_>,
                > = Box::pin(self.send_proxy(method, target_url, headers, body));
                match futures::future::select_ok(vec![relay_fut, proxy_fut]).await {
                    Ok((b, _)) => Ok(b),
                    Err(e) => Err(e),
                }
            }
            (true, false) => {
                self.send_relay(method, target_url.to_string(), headers, body)
                    .await
            }
            (false, true) => self.send_proxy(method, target_url, headers, body).await,
            (false, false) => self.send_direct(method, target_url, headers, body).await,
        }
    }

    async fn send_direct(
        &self,
        method: Method,
        target_url: &str,
        headers: HeaderMap,
        body: Option<Bytes>,
    ) -> ScResult<Bytes> {
        self.send(method, target_url, headers, body, false).await
    }

    async fn send_proxy(
        &self,
        method: Method,
        target_url: &str,
        headers: HeaderMap,
        body: Option<Bytes>,
    ) -> ScResult<Bytes> {
        self.send(method, target_url, headers, body, true).await
    }

    async fn send_relay(
        &self,
        method: Method,
        target_url: String,
        headers: HeaderMap,
        body: Option<Bytes>,
    ) -> ScResult<Bytes> {
        let relay = self
            .inner
            .relay
            .as_ref()
            .ok_or_else(|| ScError::invalid("relay not configured"))?;
        let mut h: HashMap<String, String> = HashMap::new();
        for (k, v) in headers.iter() {
            if let Ok(vs) = v.to_str() {
                h.insert(k.as_str().to_string(), vs.to_string());
            }
        }
        let req = RelayRequest {
            url: target_url,
            method: method.as_str().to_string(),
            headers: h,
            body: body.unwrap_or_default(),
        };
        let resp = relay
            .fetch(&req)
            .await
            .map_err(|e| ScError::Unreachable(e.to_string()))?;
        if resp.body.len() > RESPONSE_MAX_BYTES {
            return Err(ScError::Unreachable(
                "SoundCloud response exceeded the size limit".to_owned(),
            ));
        }
        if resp.status >= 400 {
            let retry_after_sec = resp
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(RETRY_AFTER.as_str()))
                .and_then(|(_, value)| parse_retry_after(value));
            return Err(api_error(resp.status, &resp.body, retry_after_sec));
        }
        Ok(resp.body)
    }

    async fn send(
        &self,
        method: Method,
        target_url: &str,
        headers: HeaderMap,
        body: Option<Bytes>,
        via_proxy: bool,
    ) -> ScResult<Bytes> {
        let (url, mut extra_headers) = if via_proxy && !self.inner.proxy_url.is_empty() {
            let encoded = base64::engine::general_purpose::STANDARD.encode(target_url);
            let mut h = headers;
            h.insert(
                HeaderName::from_static("x-target"),
                HeaderValue::from_str(&encoded)
                    .map_err(|e| ScError::invalid(format!("bad x-target: {e}")))?,
            );
            (self.inner.proxy_url.clone(), h)
        } else {
            (target_url.to_string(), headers)
        };

        let mut builder = self.inner.http.request(method, &url);
        for (k, v) in extra_headers.drain() {
            if let Some(name) = k {
                builder = builder.header(name, v);
            }
        }
        if let Some(b) = body {
            builder = builder.body(b);
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| ScError::Unreachable(e.without_url().to_string()))?;

        let status = resp.status();
        let retry_after_sec = resp
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after);
        let bytes = collect_capped(resp, RESPONSE_MAX_BYTES).await?;

        if status.is_client_error() || status.is_server_error() {
            return Err(api_error(status.as_u16(), &bytes, retry_after_sec));
        }

        Ok(bytes)
    }

    async fn send_direct_limited(
        &self,
        method: Method,
        target_url: &str,
        headers: HeaderMap,
        body: Option<Bytes>,
        max_bytes: usize,
    ) -> ScResult<Bytes> {
        let mut builder = self.inner.http.request(method, target_url);
        for (name, value) in headers {
            if let Some(name) = name {
                builder = builder.header(name, value);
            }
        }
        if let Some(body) = body {
            builder = builder.body(body);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| ScError::Unreachable(error.without_url().to_string()))?;
        let status = response.status();
        let retry_after_sec = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after);
        let bytes = collect_capped(response, max_bytes).await?;
        if status.is_client_error() || status.is_server_error() {
            return Err(api_error(status.as_u16(), &bytes, retry_after_sec));
        }
        Ok(bytes)
    }
}

async fn collect_capped(response: wreq::Response, max_bytes: usize) -> ScResult<Bytes> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(ScError::Unreachable(
            "SoundCloud response exceeded the size limit".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| ScError::Unreachable(error.without_url().to_string()))?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ScError::Unreachable(
                "SoundCloud response exceeded the size limit".to_owned(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(bytes))
}

fn api_error(status: u16, bytes: &[u8], retry_after_sec: Option<i64>) -> ScError {
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()))
    };
    ScError::Api {
        status,
        body,
        retry_after_sec,
    }
}

fn parse_retry_after(value: &str) -> Option<i64> {
    if let Ok(seconds) = value.trim().parse::<i64>() {
        return Some(seconds.max(1));
    }
    let retry_at = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some(
        retry_at
            .with_timezone(&chrono::Utc)
            .signed_duration_since(chrono::Utc::now())
            .num_seconds()
            .max(1),
    )
}

pub(crate) fn extract_anon_client_id(html: &str) -> Option<String> {
    HYDRATION_RE
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|id| id.as_str().to_owned())
}

fn auth_headers(access_token: &str, with_content_type: bool) -> HeaderMap {
    let mut h = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&format!("OAuth {access_token}")) {
        h.insert(AUTHORIZATION, v);
    }
    h.insert(
        ACCEPT,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    if with_content_type {
        h.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
    }
    h
}

fn build_api_url(api_base: &str, path: &str, params: Option<&[(String, String)]>) -> String {
    let base = format!("{api_base}{path}");
    match params {
        Some(p) if !p.is_empty() => {
            let qs = serde_urlencoded::to_string(p).unwrap_or_default();
            if qs.is_empty() {
                base
            } else {
                format!("{base}?{qs}")
            }
        }
        _ => base,
    }
}

pub(crate) fn decode_json<T: DeserializeOwned>(bytes: &Bytes) -> ScResult<T> {
    if bytes.is_empty() {
        return serde_json::from_slice::<T>(b"null")
            .map_err(|e| ScError::invalid(format!("empty body decode: {e}")));
    }
    serde_json::from_slice(bytes).map_err(|e| {
        tracing::warn!(error = %e, "SC JSON decode failed");
        ScError::invalid(format!("SC JSON decode: {e}"))
    })
}

fn relay_entity(answer: Option<Value>, field: &str) -> crate::RelayRead<Value> {
    let Some(value) = answer else {
        return crate::RelayRead::Unavailable;
    };
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return crate::RelayRead::Missing;
    }
    match value.get(field).cloned() {
        Some(entity) if !entity.is_null() => crate::RelayRead::Found(entity),
        _ => crate::RelayRead::Missing,
    }
}

#[cfg(test)]
mod outage_tests {
    use super::*;

    fn decoded(body: &str) -> ScResult<Value> {
        decode_json::<Value>(&Bytes::from(body.to_owned()))
    }

    #[test]
    fn a_ban_page_is_an_invalid_body_not_a_silent_success() {
        let ban = decoded("<!DOCTYPE html><html><body>Access denied</body></html>");
        assert!(matches!(ban, Err(ScError::Invalid(_))));
    }

    #[test]
    fn a_truncated_answer_is_rejected_rather_than_half_read() {
        let truncated = decoded("{\"id\": 42, \"title\": \"half");
        assert!(matches!(truncated, Err(ScError::Invalid(_))));
    }

    #[test]
    fn an_empty_answer_decodes_as_absent_instead_of_failing() {
        let empty = decode_json::<Option<Value>>(&Bytes::new());
        assert!(matches!(empty, Ok(None)));
    }

    #[test]
    fn a_well_formed_answer_still_decodes() {
        let ok = decoded("{\"id\": 42}");
        assert_eq!(ok.expect("decodes")["id"], 42);
    }

    #[test]
    fn retry_after_is_read_from_seconds_and_ignored_when_nonsense() {
        assert_eq!(parse_retry_after("30"), Some(30));
        assert_eq!(
            parse_retry_after("0"),
            Some(1),
            "a zero wait must still be a wait"
        );
        assert_eq!(parse_retry_after("soon"), None);
        assert_eq!(parse_retry_after(""), None);
    }
}

#[cfg(test)]
mod size_tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    async fn serve(head: String, body: Vec<u8>) -> std::io::Result<(String, JoinHandle<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await;
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(&body).await;
            let _ = stream.shutdown().await;
        });
        Ok((format!("http://{address}/probe"), server))
    }

    fn client(api_base: &str) -> ScClient {
        ScClient::new(&crate::ScConfig {
            proxy_url: String::new(),
            proxy_fallback: false,
            api_base: Some(api_base.to_owned()),
            home_base: None,
        })
        .expect("builds")
    }

    fn oversized_is_reported(error: &ScError) -> bool {
        matches!(error, ScError::Unreachable(message) if message.contains("exceeded the size limit"))
    }

    #[tokio::test]
    async fn a_declared_length_over_the_cap_is_refused_before_the_body_arrives() -> TestResult {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            RESPONSE_MAX_BYTES + 1
        );
        let (url, server) = serve(head, Vec::new()).await?;
        let sc = client("http://127.0.0.1:1");

        let result = tokio::time::timeout(
            TEST_TIMEOUT,
            sc.send(Method::GET, &url, HeaderMap::new(), None, false),
        )
        .await?;

        server.abort();
        let error = result.expect_err("an oversized answer must not be read");
        assert!(oversized_is_reported(&error), "{error}");
        Ok(())
    }

    #[tokio::test]
    async fn a_body_that_outgrows_the_cap_while_streaming_is_refused() -> TestResult {
        let head = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
            .to_owned();
        let (url, server) = serve(head, vec![b'x'; RESPONSE_MAX_BYTES + 4096]).await?;
        let sc = client("http://127.0.0.1:1");

        let result = tokio::time::timeout(
            TEST_TIMEOUT,
            sc.send(Method::GET, &url, HeaderMap::new(), None, false),
        )
        .await?;

        server.abort();
        let error = result.expect_err("a body without a declared length must still be capped");
        assert!(oversized_is_reported(&error), "{error}");
        Ok(())
    }

    #[tokio::test]
    async fn a_token_answer_keeps_its_own_tighter_cap() -> TestResult {
        let head = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
            .to_owned();
        let (url, server) = serve(head, vec![b'x'; TOKEN_RESPONSE_MAX_BYTES + 1024]).await?;
        let sc = client("http://127.0.0.1:1");

        let result = tokio::time::timeout(
            TEST_TIMEOUT,
            sc.send_direct_limited(
                Method::POST,
                &url,
                HeaderMap::new(),
                None,
                TOKEN_RESPONSE_MAX_BYTES,
            ),
        )
        .await?;

        server.abort();
        let error = result.expect_err("a token answer must stay small");
        assert!(oversized_is_reported(&error), "{error}");
        Ok(())
    }

    #[tokio::test]
    async fn an_answer_inside_the_cap_still_arrives_whole() -> TestResult {
        let payload = serde_json::to_vec(&serde_json::json!({ "id": 42 }))?;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            payload.len()
        );
        let (url, server) = serve(head, payload).await?;
        let sc = client("http://127.0.0.1:1");

        let bytes = tokio::time::timeout(
            TEST_TIMEOUT,
            sc.send(Method::GET, &url, HeaderMap::new(), None, false),
        )
        .await??;

        server.abort();
        assert_eq!(decode_json::<Value>(&bytes)?["id"], 42);
        Ok(())
    }

    const SAMPLE_HYDRATION: &str = r#"window.__sc_hydration = [{"hydratable":"apiClient","data":{"id":"JNsHQvoXu3CrVm6Jv30i95VRZQ7h8lXX","isExpiring":false}}];"#;

    async fn serve_homepage(
        answer: &'static str,
        delay: Duration,
    ) -> std::io::Result<(String, Arc<std::sync::atomic::AtomicUsize>, JoinHandle<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let visits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = visits.clone();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let answer = answer.to_owned();
                tokio::spawn(async move {
                    let mut request = [0_u8; 4096];
                    let _ = stream.read(&mut request).await;
                    tokio::time::sleep(delay).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n",
                        answer.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(answer.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        Ok((format!("http://{address}"), visits, server))
    }

    fn homepage_client(home_base: &str) -> ScClient {
        ScClient::new(&crate::ScConfig {
            proxy_url: String::new(),
            proxy_fallback: false,
            api_base: Some("http://127.0.0.1:1".to_owned()),
            home_base: Some(home_base.to_owned()),
        })
        .expect("builds")
    }

    #[tokio::test]
    async fn a_crowd_of_callers_reads_the_homepage_once() -> TestResult {
        let (home, visits, server) =
            serve_homepage(SAMPLE_HYDRATION, Duration::from_millis(150)).await?;
        let sc = homepage_client(&home);

        let callers: Vec<_> = (0..8)
            .map(|_| {
                let sc = sc.clone();
                tokio::spawn(async move { sc.anon_client_id().await })
            })
            .collect();
        let mut ids = Vec::new();
        for caller in callers {
            ids.push(caller.await?);
        }

        server.abort();
        assert_eq!(
            visits.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "concurrent callers must collapse into one homepage read"
        );
        assert!(
            ids.iter()
                .all(|id| id.as_deref() == Some("JNsHQvoXu3CrVm6Jv30i95VRZQ7h8lXX")),
            "{ids:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_homepage_without_a_client_id_is_not_asked_again_immediately() -> TestResult {
        let (home, visits, server) =
            serve_homepage("<html>no hydration here</html>", Duration::ZERO).await?;
        let sc = homepage_client(&home);

        assert_eq!(sc.anon_client_id().await, None);
        assert_eq!(sc.anon_client_id().await, None);
        assert_eq!(sc.rotate_anon_client_id(None).await, None);

        server.abort();
        assert_eq!(
            visits.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a failed homepage read must back off instead of retrying on every call"
        );
        Ok(())
    }

    #[test]
    fn the_backoff_grows_and_stops_growing() {
        let mut state = AnonClientId::default();
        assert!(!state.is_backing_off());
        state.reject();
        assert!(state.is_backing_off());
        let first = state.retry_at.expect("a wait");
        for _ in 0..20 {
            state.reject();
        }
        let capped = state.retry_at.expect("a wait");
        assert!(capped > first);
        assert!(capped <= Instant::now() + ANON_CID_RETRY_MAX);
        state.accept("id".to_owned());
        assert!(!state.is_backing_off());
        assert_eq!(state.fresh().as_deref(), Some("id"));
    }

    #[test]
    fn the_client_id_is_read_out_of_the_hydration_block() {
        assert_eq!(
            extract_anon_client_id(SAMPLE_HYDRATION).as_deref(),
            Some("JNsHQvoXu3CrVm6Jv30i95VRZQ7h8lXX")
        );
        assert_eq!(extract_anon_client_id("<html></html>"), None);
    }

    struct ScriptedRelay {
        answer: Option<&'static str>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl crate::RelayTransport for ScriptedRelay {
        fn fetch<'a>(
            &'a self,
            _request: &'a RelayRequest,
        ) -> super::RelayFuture<'a, call_relay::Response> {
            Box::pin(async { Err(call_relay::Error::Disabled) })
        }

        fn call_method_rotated<'a>(
            &'a self,
            _method_id: &'a str,
            _script: &'a str,
            _inputs: Bytes,
            _region_rotation: i32,
        ) -> super::RelayFuture<'a, Bytes> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let answer = self.answer;
            Box::pin(async move {
                match answer {
                    Some(body) => Ok(Bytes::from_static(body.as_bytes())),
                    None => Err(call_relay::Error::Disabled),
                }
            })
        }
    }

    fn relayed(answer: Option<&'static str>) -> (ScClient, Arc<std::sync::atomic::AtomicUsize>) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let relay = Arc::new(ScriptedRelay {
            answer,
            calls: calls.clone(),
        });
        let sc = ScClient::new(&crate::ScConfig {
            proxy_url: String::new(),
            proxy_fallback: false,
            api_base: Some("http://127.0.0.1:1".to_owned()),
            home_base: Some("http://127.0.0.1:1".to_owned()),
        })
        .expect("builds")
        .with_relay(relay);
        (sc, calls)
    }

    #[tokio::test]
    async fn a_relay_that_answers_hands_the_entity_back() -> TestResult {
        let (sc, calls) = relayed(Some(r#"{"ok":true,"track":{"id":42,"kind":"track"}}"#));

        let read = sc.track_by_id_via_relay("42").await;

        assert!(
            matches!(&read, crate::RelayRead::Found(value) if value["id"] == 42),
            "{read:?}"
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn a_relay_that_says_the_entity_is_absent_is_a_miss_and_not_an_outage() -> TestResult {
        let (sc, _) = relayed(Some(r#"{"ok":true,"track":null}"#));
        assert!(matches!(
            sc.track_by_id_via_relay("42").await,
            crate::RelayRead::Missing
        ));

        let (refused, _) = relayed(Some(r#"{"ok":false}"#));
        assert!(
            matches!(
                refused.track_by_id_via_relay("42").await,
                crate::RelayRead::Missing
            ),
            "a relay that answered but refused is still an answer, not an outage"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_silent_relay_reads_as_unavailable_rather_than_absent() -> TestResult {
        let (sc, calls) = relayed(None);

        assert!(matches!(
            sc.track_by_id_via_relay("42").await,
            crate::RelayRead::Unavailable
        ));
        assert!(calls.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        Ok(())
    }

    #[tokio::test]
    async fn a_deletion_that_answers_with_a_ban_page_is_not_a_silent_success() -> TestResult {
        let body = b"<!DOCTYPE html><html><body>Access denied</body></html>".to_vec();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let (url, server) = serve(head, body).await?;
        let base = url.trim_end_matches("/probe").to_owned();
        let sc = client(&base);

        let result = tokio::time::timeout(TEST_TIMEOUT, sc.api_delete("/probe", "token")).await?;

        server.abort();
        assert!(
            matches!(result, Err(ScError::Invalid(_))),
            "a ban page must not read as a finished deletion"
        );
        Ok(())
    }
}

#[cfg(test)]
mod redaction_tests {
    use super::*;
    use tokio::net::TcpListener;

    const RESOURCE_SECRET: &str = "s3cr3t-resource-capability";
    const ACCESS_TOKEN: &str = "s3cr3t-access-token";

    async fn a_port_nobody_answers_on() -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port can be reserved");
        let address = listener.local_addr().expect("the port has an address");
        drop(listener);
        format!("http://{address}")
    }

    #[tokio::test]
    async fn a_transport_failure_never_repeats_the_capability_it_was_carrying() {
        let dead = a_port_nobody_answers_on().await;
        let client = ScClient::new(&ScConfig {
            proxy_url: String::new(),
            proxy_fallback: false,
            api_base: Some(dead.clone()),
            home_base: Some(dead),
        })
        .expect("the client builds");

        let params = [("secret_token".to_owned(), RESOURCE_SECRET.to_owned())];
        let failure = client
            .api_get_value("/tracks/42", ACCESS_TOKEN, Some(&params))
            .await
            .expect_err("nothing is listening on that port");

        let told = failure.to_string();
        assert!(
            !told.contains(RESOURCE_SECRET),
            "a transport error prints the url it failed on, and that url carries the private \
             track capability; this text is logged at every direct-read failure: {told}"
        );
        assert!(
            !told.contains(ACCESS_TOKEN),
            "the access token travels in a header, so it must not appear either: {told}"
        );
        assert!(
            !told.is_empty(),
            "stripping the url must not leave an error nobody can act on"
        );
    }
}
