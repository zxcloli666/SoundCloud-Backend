use std::sync::Arc;

use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::ChatState;
use crate::modules::chat::conversations::fixtures::{
    DEVICE, access_of, group, key_epoch, outgoing,
};
use crate::modules::chat::media::refs;
use crate::modules::chat::media::rows::{Seed, seed, status, usage};
use crate::modules::chat::media::support::{
    IMAGE, Rig, StoreSpec, THUMB, bench_with, call, error, id_of, image_body, reserve, star, user,
};
use crate::modules::chat::messages::store;
use crate::modules::chat::model::MemberState;

const TOTAL: i64 = (IMAGE + THUMB) as i64;

fn env(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: these tests need a live SeaweedFS"))
}

async fn rig(stores: &[u16], write: u16) -> Rig {
    let endpoint = env("CHAT_TEST_S3_ENDPOINT");
    let (access_key, secret) = (
        env("CHAT_TEST_S3_ACCESS_KEY"),
        env("CHAT_TEST_S3_SECRET_KEY"),
    );
    let admin = aws_sdk_s3::Client::from_conf(
        aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(&endpoint)
            .credentials_provider(Credentials::new(
                &access_key,
                &secret,
                None,
                None,
                "chat-live",
            ))
            .force_path_style(true)
            .build(),
    );
    let mut specs = Vec::new();
    for id in stores {
        let bucket = format!(
            "chat-live-{id}-{}",
            &Uuid::new_v4().simple().to_string()[..12]
        );
        admin
            .create_bucket()
            .bucket(&bucket)
            .send()
            .await
            .expect("a bucket");
        specs.push(StoreSpec {
            id: *id,
            bucket,
            cap_gb: 200,
        });
    }
    Rig {
        presign: endpoint.clone(),
        endpoint,
        access_key,
        secret,
        stores: specs,
        write,
    }
}

fn bytes_of(length: u64, fill: u8) -> Vec<u8> {
    vec![fill; usize::try_from(length).expect("fits")]
}

async fn put(target: &Value, body: Vec<u8>) -> u16 {
    let mut request = wreq::Client::new().put(target["url"].as_str().expect("a url"));
    for (name, value) in target["headers"].as_object().expect("headers") {
        request = request.header(name.as_str(), value.as_str().expect("a header value"));
    }
    request
        .body(body)
        .send()
        .await
        .expect("a put")
        .status()
        .as_u16()
}

async fn fetch(url: &str) -> (u16, Vec<u8>) {
    let answer = wreq::Client::new().get(url).send().await.expect("a get");
    let status = answer.status().as_u16();
    (status, answer.bytes().await.expect("a body").to_vec())
}

async fn url_of(chat: &Arc<ChatState>, user: &str, body: Value) -> String {
    let answer = call(chat, user, "POST", "/chat/media/urls", Some(body)).await;
    assert_eq!(answer.1["items"][0]["state"], "ok", "{answer:?}");
    answer.1["items"][0]["url"]
        .as_str()
        .expect("a url")
        .to_owned()
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs a live SeaweedFS (CHAT_TEST_S3_ENDPOINT)"]
async fn an_image_goes_reserve_put_commit_and_comes_back_through_urls(pool: PgPool) {
    let chat = bench_with(&pool, &rig(&[1], 1).await);
    let (owner, member) = (user(), user());
    star(&pool, &owner).await;
    let group = group(&pool, &owner, &[(&member, MemberState::Active)]).await;
    let answer = reserve(&chat, &owner, image_body(group)).await;
    let (image, thumb) = (id_of(&answer, "id"), id_of(&answer["thumb"], "id"));
    assert_eq!(put(&answer["put"], bytes_of(IMAGE, 0x41)).await, 200);
    assert_eq!(
        put(&answer["thumb"]["put"], bytes_of(THUMB, 0x42)).await,
        200
    );
    let commit = call(
        &chat,
        &owner,
        "POST",
        &format!("/chat/media/{image}/commit"),
        None,
    )
    .await;
    assert_eq!(commit.0, 200, "{commit:?}");
    assert_eq!(commit.1["bytes"].as_i64(), Some(TOTAL));
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
    let again = call(
        &chat,
        &owner,
        "POST",
        &format!("/chat/media/{image}/commit"),
        None,
    )
    .await;
    assert_eq!(again, commit);
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
    let own = url_of(&chat, &owner, json!({"ids": [image]})).await;
    assert_eq!(fetch(&own).await, (200, bytes_of(IMAGE, 0x41)));
    let access = access_of(&pool, group, &owner).await;
    let mut out = outgoing(
        sc_chat_wire::msg::Class::Post,
        key_epoch(&pool, group).await,
        None,
    );
    out.media = vec![image];
    refs::admit_send(&chat, &access, &out)
        .await
        .expect("admitted");
    store::store(&pool, &access, DEVICE, &out, None)
        .await
        .expect("a post");
    assert_eq!(status(&pool, image).await.as_deref(), Some("attached"));
    let shared = url_of(
        &chat,
        &member,
        json!({"conversation_id": group, "ids": [thumb]}),
    )
    .await;
    assert_eq!(fetch(&shared).await, (200, bytes_of(THUMB, 0x42)));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs a live SeaweedFS (CHAT_TEST_S3_ENDPOINT)"]
async fn a_missing_or_short_object_names_the_ids_to_put_again(pool: PgPool) {
    let chat = bench_with(&pool, &rig(&[1], 1).await);
    let owner = user();
    star(&pool, &owner).await;
    let group = group(&pool, &owner, &[]).await;
    let body = image_body(group);
    let answer = reserve(&chat, &owner, body.clone()).await;
    let (image, thumb) = (id_of(&answer, "id"), id_of(&answer["thumb"], "id"));
    let uri = format!("/chat/media/{image}/commit");
    let none = call(&chat, &owner, "POST", &uri, None).await;
    assert_eq!(error(&none), (409, "chat_media_missing"));
    assert_eq!(none.1["ids"], json!([image, thumb]));
    assert_eq!(put(&answer["thumb"]["put"], bytes_of(THUMB, 1)).await, 200);
    let mut wrong = answer["put"].clone();
    wrong["headers"]["content-length"] = json!((IMAGE - 4096).to_string());
    assert_ne!(put(&wrong, bytes_of(IMAGE - 4096, 2)).await, 200);
    let key: String = sqlx::query_scalar("SELECT object_key FROM chat.media WHERE id = $1")
        .bind(image)
        .fetch_one(&pool)
        .await
        .expect("a key");
    chat.s3
        .put(1, &key, bytes_of(IMAGE - 4096, 3))
        .await
        .expect("a short object");
    let short = call(&chat, &owner, "POST", &uri, None).await;
    assert_eq!(error(&short), (409, "chat_media_missing"));
    assert_eq!(short.1["ids"], json!([image]));
    assert_eq!(usage(&pool, &owner).await, (0, TOTAL, 0));
    let replay = reserve(&chat, &owner, body).await;
    assert_eq!(id_of(&replay, "id"), image);
    assert_eq!(put(&replay["put"], bytes_of(IMAGE, 4)).await, 200);
    let done = call(&chat, &owner, "POST", &uri, None).await;
    assert_eq!(done.0, 200, "{done:?}");
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs a live SeaweedFS (CHAT_TEST_S3_ENDPOINT)"]
async fn writes_land_on_the_write_store_and_reads_use_the_store_of_the_row(pool: PgPool) {
    let rig = rig(&[1, 2], 2).await;
    let chat = bench_with(&pool, &rig);
    let owner = user();
    star(&pool, &owner).await;
    let group = group(&pool, &owner, &[]).await;
    let (old, _) = seed(&pool, Seed::image(&owner, group, "attached")).await;
    let old_key: String = sqlx::query_scalar("SELECT object_key FROM chat.media WHERE id = $1")
        .bind(old)
        .fetch_one(&pool)
        .await
        .expect("a key");
    chat.s3
        .put(1, &old_key, bytes_of(IMAGE, 7))
        .await
        .expect("an old object");
    let answer = reserve(&chat, &owner, image_body(group)).await;
    let new = id_of(&answer, "id");
    assert!(
        answer["put"]["url"]
            .as_str()
            .unwrap_or_default()
            .contains(&rig.stores[1].bucket)
    );
    assert_eq!(put(&answer["put"], bytes_of(IMAGE, 8)).await, 200);
    assert_eq!(put(&answer["thumb"]["put"], bytes_of(THUMB, 9)).await, 200);
    let commit = call(
        &chat,
        &owner,
        "POST",
        &format!("/chat/media/{new}/commit"),
        None,
    )
    .await;
    assert_eq!(commit.0, 200, "{commit:?}");
    let stores: Vec<i16> =
        sqlx::query_scalar("SELECT store FROM chat.media WHERE id = $1 OR parent_id = $1")
            .bind(new)
            .fetch_all(&pool)
            .await
            .expect("stores");
    assert_eq!(stores, [2, 2]);
    let old_url = url_of(&chat, &owner, json!({"ids": [old]})).await;
    assert!(old_url.contains(&rig.stores[0].bucket), "{old_url}");
    assert_eq!(fetch(&old_url).await, (200, bytes_of(IMAGE, 7)));
    let new_url = url_of(&chat, &owner, json!({"ids": [new]})).await;
    assert_eq!(fetch(&new_url).await, (200, bytes_of(IMAGE, 8)));
    assert_eq!(
        chat.s3.head(1, &old_key).await.expect("a head"),
        Some(IMAGE)
    );
}
