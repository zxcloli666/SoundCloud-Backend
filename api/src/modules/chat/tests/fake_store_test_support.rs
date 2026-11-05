use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::modules::chat::media::MediaConfig;

const COPY_SOURCE: &str = "x-amz-copy-source";
const COPIED: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CopyObjectResult><LastModified>2026-01-01T00:00:00.000Z</LastModified><ETag>\"fake\"</ETag></CopyObjectResult>";

#[derive(Default)]
struct Shelf {
    objects: Mutex<HashMap<String, Vec<u8>>>,
    refused: Mutex<HashSet<String>>,
    asked: Mutex<Vec<(Method, String)>>,
    copies_fail: AtomicBool,
    delay: Duration,
}

#[derive(Clone)]
pub struct FakeStore {
    pub endpoint: String,
    shelf: Arc<Shelf>,
}

impl FakeStore {
    pub async fn start(delay: Duration) -> Self {
        let shelf = Arc::new(Shelf {
            delay,
            ..Shelf::default()
        });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a fake store port");
        let endpoint = format!("http://{}", listener.local_addr().expect("an address"));
        let app = Router::new().fallback(answer).with_state(shelf.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        Self { endpoint, shelf }
    }

    pub fn put(&self, key: &str, bytes: Vec<u8>) {
        self.shelf
            .objects
            .lock()
            .expect("the shelf")
            .insert(key.to_owned(), bytes);
    }

    pub fn object(&self, key: &str) -> Option<Vec<u8>> {
        self.shelf
            .objects
            .lock()
            .expect("the shelf")
            .get(key)
            .cloned()
    }

    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .shelf
            .objects
            .lock()
            .expect("the shelf")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    pub fn fail_copies(&self, failing: bool) {
        self.shelf.copies_fail.store(failing, Ordering::SeqCst);
    }

    pub fn refuse(&self, key: &str) {
        self.shelf
            .refused
            .lock()
            .expect("the shelf")
            .insert(key.to_owned());
    }

    pub fn allow(&self, key: &str) {
        self.shelf.refused.lock().expect("the shelf").remove(key);
    }

    pub fn asked(&self, method: Method) -> Vec<String> {
        self.shelf
            .asked
            .lock()
            .expect("the shelf")
            .iter()
            .filter(|(asked, _)| *asked == method)
            .map(|(_, key)| key.clone())
            .collect()
    }
}

async fn answer(
    State(shelf): State<Arc<Shelf>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    tokio::time::sleep(shelf.delay).await;
    let key = uri
        .path()
        .trim_start_matches('/')
        .split_once('/')
        .map(|(_, key)| key.to_owned())
        .unwrap_or_default();
    shelf
        .asked
        .lock()
        .expect("the shelf")
        .push((method.clone(), key.clone()));
    if shelf.refused.lock().expect("the shelf").contains(&key) {
        return refusal(StatusCode::FORBIDDEN, "AccessDenied");
    }
    let mut objects = shelf.objects.lock().expect("the shelf");
    match method {
        Method::GET => match objects.get(&key) {
            Some(bytes) => bytes.clone().into_response(),
            None => refusal(StatusCode::NOT_FOUND, "NoSuchKey"),
        },
        Method::HEAD => match objects.get(&key) {
            Some(bytes) => (
                StatusCode::OK,
                [(header::CONTENT_LENGTH, bytes.len().to_string())],
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        Method::PUT if headers.contains_key(COPY_SOURCE) => {
            if shelf.copies_fail.load(Ordering::SeqCst) {
                return refusal(StatusCode::INTERNAL_SERVER_ERROR, "InternalError");
            }
            let source = headers
                .get(COPY_SOURCE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim_start_matches('/').split_once('/'))
                .map(|(_, source)| source.to_owned())
                .unwrap_or_default();
            match objects.get(&source).cloned() {
                Some(bytes) => {
                    objects.insert(key, bytes);
                    (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "application/xml")],
                        COPIED,
                    )
                        .into_response()
                }
                None => refusal(StatusCode::NOT_FOUND, "NoSuchKey"),
            }
        }
        Method::PUT => {
            let chunked = headers
                .get(header::CONTENT_ENCODING)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.contains("aws-chunked"));
            let bytes = if chunked {
                unchunk(&body)
            } else {
                body.to_vec()
            };
            objects.insert(key, bytes);
            (StatusCode::OK, [(header::ETAG, "\"fake\"")]).into_response()
        }
        Method::DELETE => {
            objects.remove(&key);
            StatusCode::NO_CONTENT.into_response()
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

fn refusal(status: StatusCode, code: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/xml")],
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code><Message>{code}</Message></Error>"),
    )
        .into_response()
}

fn unchunk(mut body: &[u8]) -> Vec<u8> {
    let mut plain = Vec::new();
    while let Some(end) = body.windows(2).position(|pair| pair == b"\r\n") {
        let size = std::str::from_utf8(&body[..end])
            .ok()
            .and_then(|line| line.split(';').next())
            .and_then(|hex| usize::from_str_radix(hex.trim(), 16).ok())
            .unwrap_or(0);
        body = &body[end + 2..];
        if size == 0 || body.len() < size {
            break;
        }
        plain.extend_from_slice(&body[..size]);
        body = body.get(size + 2..).unwrap_or_default();
    }
    plain
}

pub fn media_on(stores: &[(u16, &FakeStore)]) -> MediaConfig {
    let dir = std::env::temp_dir().join(format!("chat-fake-store-{}", Uuid::now_v7().simple()));
    std::fs::create_dir_all(&dir).expect("a keys dir");
    for (id, _) in stores {
        std::fs::write(dir.join(format!("s3-secret-{id}")), "fake-secret").expect("a secret");
    }
    let media = parse(stores, &dir);
    std::fs::remove_dir_all(&dir).ok();
    media
}

fn parse(stores: &[(u16, &FakeStore)], dir: &Path) -> MediaConfig {
    let list: Vec<String> = stores.iter().map(|(id, _)| id.to_string()).collect();
    let list = list.join(",");
    let write = stores.first().map(|(id, _)| id.to_string());
    MediaConfig::parse(
        |name| {
            if name == "CHAT_STORES" {
                return Some(list.clone());
            }
            if name == "CHAT_STORE_WRITE" {
                return write.clone();
            }
            stores.iter().find_map(|(id, store)| {
                let field = name.strip_prefix(&format!("CHAT_STORE_{id}_"))?;
                match field {
                    "ENDPOINT" | "PRESIGN" => Some(store.endpoint.clone()),
                    "BUCKET" => Some(format!("fake-{id}")),
                    "ACCESS_KEY" => Some("fake".to_owned()),
                    "CAP_GB" => Some("200".to_owned()),
                    _ => None,
                }
            })
        },
        dir,
    )
    .expect("a fake media config")
}
