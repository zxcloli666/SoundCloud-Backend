use std::time::Duration;

use super::S3;
use crate::modules::chat::ChatError;
use crate::modules::chat::media::MediaConfig;

fn s3() -> S3 {
    let dir = std::env::temp_dir().join(format!("chat-s3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a dir");
    std::fs::write(dir.join("s3-secret-1"), "not-a-real-secret").expect("a secret");
    let pairs = [
        ("CHAT_STORES", "1"),
        ("CHAT_STORE_1_ENDPOINT", "https://s3.int.scnative.dev:8443"),
        ("CHAT_STORE_1_PRESIGN", "https://s3.scnative.dev"),
        ("CHAT_STORE_1_BUCKET", "scd-chat"),
        ("CHAT_STORE_1_ACCESS_KEY", "AKIATEST"),
    ];
    S3::new(&MediaConfig::parse(
        |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        },
        &dir,
    ))
}

#[tokio::test]
async fn a_put_is_signed_for_the_public_host_one_length_and_one_type() {
    let put = s3()
        .presign_put(1, "m/abc", 151_552, Duration::from_secs(600))
        .await
        .expect("a presigned put");
    assert!(
        put.url
            .starts_with("https://s3.scnative.dev/scd-chat/m/abc?"),
        "{}",
        put.url
    );
    assert!(put.url.contains("X-Amz-Expires=600"), "{}", put.url);
    let signed = put
        .url
        .split("X-Amz-SignedHeaders=")
        .nth(1)
        .and_then(|rest| rest.split('&').next())
        .map(|headers| headers.replace("%3B", ";"))
        .expect("signed headers");
    for header in ["host", "content-length", "content-type"] {
        assert!(signed.split(';').any(|one| one == header), "{signed}");
    }
    assert_eq!(put.headers["content-length"], "151552");
    assert_eq!(put.headers["content-type"], "application/octet-stream");
    assert_eq!(put.headers.len(), 2);
}

#[tokio::test]
async fn a_get_lives_fifteen_minutes_and_never_names_the_internal_endpoint() {
    let url = s3()
        .presign_get(1, "m/abc", Duration::from_secs(900))
        .await
        .expect("a presigned get");
    assert!(
        url.starts_with("https://s3.scnative.dev/scd-chat/m/abc?"),
        "{url}"
    );
    assert!(url.contains("X-Amz-Expires=900"), "{url}");
    assert!(!url.contains("int.scnative.dev"));
}

#[tokio::test]
async fn a_store_nobody_configured_is_a_server_fault() {
    assert_eq!(
        s3().presign_get(7, "m/abc", Duration::from_secs(900)).await,
        Err(ChatError::Internal)
    );
    let empty = S3::new(&MediaConfig::default());
    assert_eq!(
        empty
            .presign_put(1, "k", 4096, Duration::from_secs(600))
            .await,
        Err(ChatError::Internal)
    );
}
