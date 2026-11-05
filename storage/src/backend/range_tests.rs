use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use futures::StreamExt;

use super::{BackendError, ByteStream, LocalBackend, Ranged, S3Backend};
use crate::byte_range::{ByteRange, Slice};
use crate::config::S3Config;

const SIZE: usize = 1000;

fn object() -> Vec<u8> {
    (0..SIZE).map(|i| (i % 251) as u8).collect()
}

#[derive(Default)]
struct Calls {
    heads: AtomicUsize,
    gets: AtomicUsize,
}

fn xml_error(status: StatusCode, code: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code><Message>x</Message></Error>"
        )))
        .unwrap()
}

async fn fake_get(
    State(calls): State<Arc<Calls>>,
    Path((_, key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    calls.gets.fetch_add(1, Ordering::SeqCst);
    let data = object();
    if key == "plain.m4a" {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_LENGTH, SIZE)
            .body(Body::from(data))
            .unwrap();
    }
    if key != "track.m4a" {
        return xml_error(StatusCode::NOT_FOUND, "NoSuchKey");
    }
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(ByteRange::parse);
    match range.and_then(|range| range.slice(SIZE as u64)) {
        Some(slice) => Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, slice.content_range())
            .header(header::CONTENT_LENGTH, slice.len())
            .body(Body::from(
                data[slice.start as usize..=slice.end as usize].to_vec(),
            ))
            .unwrap(),
        None => xml_error(StatusCode::RANGE_NOT_SATISFIABLE, "InvalidRange"),
    }
}

async fn fake_head(
    State(calls): State<Arc<Calls>>,
    Path((_, key)): Path<(String, String)>,
) -> Response {
    calls.heads.fetch_add(1, Ordering::SeqCst);
    let status = if key == "track.m4a" || key == "plain.m4a" {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };
    Response::builder()
        .status(status)
        .header(header::CONTENT_LENGTH, SIZE)
        .body(Body::empty())
        .unwrap()
}

async fn fake_s3() -> (S3Backend, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let app = Router::new()
        .route("/{bucket}/{*key}", get(fake_get).head(fake_head))
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    let backend = S3Backend::new(&S3Config {
        endpoint: Some(format!("http://{address}")),
        presign_endpoint: None,
        region: "us-east-1".to_owned(),
        bucket: "tracks".to_owned(),
        access_key_id: "test".to_owned(),
        secret_access_key: "test".to_owned(),
        force_path_style: true,
    })
    .await;
    (backend, calls)
}

async fn bytes_of(stream: ByteStream) -> Vec<u8> {
    stream.map(|chunk| chunk.unwrap().to_vec()).concat().await
}

fn range(value: &str) -> ByteRange {
    ByteRange::parse(value).unwrap()
}

#[tokio::test]
async fn s3_takes_the_total_from_the_ranged_get_without_a_head() {
    let (s3, calls) = fake_s3().await;
    for (asked, start, end) in [
        ("bytes=100-199", 100, 199),
        ("bytes=900-", 900, 999),
        ("bytes=-50", 950, 999),
        ("bytes=990-5000", 990, 999),
    ] {
        let Ranged::Part { slice, stream, .. } =
            s3.read_range("track.m4a", &range(asked)).await.unwrap()
        else {
            panic!("{asked} is not a part");
        };
        assert_eq!(
            slice,
            Slice {
                start,
                end,
                total: SIZE as u64
            },
            "{asked}"
        );
        assert_eq!(
            bytes_of(stream).await,
            object()[start as usize..=end as usize].to_vec()
        );
    }
    assert_eq!(calls.heads.load(Ordering::SeqCst), 0);
    assert_eq!(calls.gets.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn s3_heads_only_after_a_refused_range() {
    let (s3, calls) = fake_s3().await;
    let refused = s3.read_range("track.m4a", &range("bytes=1000-")).await;
    assert!(matches!(refused, Ok(Ranged::Unsatisfiable { total: 1000 })));
    assert_eq!(calls.heads.load(Ordering::SeqCst), 1);

    let empty = s3.read_range("track.m4a", &range("bytes=-0")).await;
    assert!(matches!(empty, Ok(Ranged::Unsatisfiable { total: 1000 })));
    assert_eq!(calls.heads.load(Ordering::SeqCst), 2);
    assert_eq!(
        calls.gets.load(Ordering::SeqCst),
        1,
        "an empty suffix never goes to S3 as a get"
    );
}

#[tokio::test]
async fn s3_without_range_support_gives_the_whole_object() {
    let (s3, _) = fake_s3().await;
    let Ranged::Whole { info, stream } = s3
        .read_range("plain.m4a", &range("bytes=0-9"))
        .await
        .unwrap()
    else {
        panic!("not a whole object");
    };
    assert_eq!(info.size, SIZE as u64);
    assert_eq!(bytes_of(stream).await, object());
}

#[tokio::test]
async fn s3_missing_key_is_not_found() {
    let (s3, _) = fake_s3().await;
    let missing = s3.read_range("gone.m4a", &range("bytes=0-9")).await;
    assert!(matches!(missing, Err(BackendError::NotFound)));
}

#[tokio::test]
async fn local_reads_a_slice_and_refuses_past_the_end() {
    let root = std::env::temp_dir().join(format!("storage-range-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&root).await.unwrap();
    tokio::fs::write(root.join("track.m4a"), object())
        .await
        .unwrap();
    let local = LocalBackend::new(root.to_str().unwrap()).await.unwrap();

    let Ranged::Part { slice, stream, .. } = local
        .read_range("track.m4a", &range("Bytes=10-19"))
        .await
        .unwrap()
    else {
        panic!("not a part");
    };
    assert_eq!(slice.content_range(), "bytes 10-19/1000");
    assert_eq!(bytes_of(stream).await, object()[10..20].to_vec());
    assert!(matches!(
        local.read_range("track.m4a", &range("bytes=1000-")).await,
        Ok(Ranged::Unsatisfiable { total: 1000 })
    ));
    assert!(matches!(
        local.read_range("gone.m4a", &range("bytes=0-1")).await,
        Err(BackendError::NotFound)
    ));
    tokio::fs::remove_dir_all(&root).await.unwrap();
}
