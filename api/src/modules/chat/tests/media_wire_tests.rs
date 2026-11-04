use serde_json::{Value, json};
use uuid::Uuid;

use super::*;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn parsed<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("the contract shape parses")
}

#[test]
fn reserve_bodies_parse_the_contract_json() {
    let image: ReserveBody = parsed(json!({
        "upload_id": id(1), "conversation_id": id(2),
        "kind": "image", "bytes": 151_552, "thumb_bytes": 24_576
    }));
    assert_eq!(image.kind, MediaKind::Image);
    assert_eq!(image.thumb_bytes, Some(24_576));
    image.validate().expect("a valid image reserve");
    let avatar: ReserveBody = parsed(json!({
        "upload_id": id(1), "conversation_id": id(2), "kind": "avatar", "bytes": 16_384
    }));
    assert_eq!(avatar.thumb_bytes, None);
    avatar.validate().expect("a valid avatar reserve");
}

#[test]
fn a_reserve_that_breaks_the_shape_is_invalid() {
    let make = |kind, bytes, thumb| ReserveBody {
        upload_id: id(1),
        conversation_id: id(2),
        kind,
        bytes,
        thumb_bytes: thumb,
    };
    let bad = [
        make(MediaKind::Image, 151_552, None),
        make(MediaKind::Avatar, 16_384, Some(4_096)),
        make(MediaKind::Image, 0, Some(4_096)),
        make(MediaKind::Image, 151_553, Some(4_096)),
        make(MediaKind::Image, 614_400 + 4_096, Some(4_096)),
        make(MediaKind::Image, 614_400, Some(32_768 + 4_096)),
        make(MediaKind::Image, 8_192, Some(0)),
        make(MediaKind::Avatar, 65_536 + 4_096, None),
    ];
    for body in bad {
        assert_eq!(body.validate(), Err(ChatError::Invalid), "{body:?}");
    }
    make(MediaKind::Image, 614_400, Some(32_768))
        .validate()
        .expect("the largest image");
    make(MediaKind::Avatar, 65_536, None)
        .validate()
        .expect("the largest avatar");
}

#[test]
fn the_unknown_thumb_kind_is_not_accepted() {
    let parsed: Result<ReserveBody, _> = serde_json::from_value(json!({
        "upload_id": id(1), "conversation_id": id(2), "kind": "thumb", "bytes": 4096
    }));
    assert!(parsed.is_err());
}

#[test]
fn the_reserve_answer_matches_the_core_wire() {
    let answer = ReserveAnswer {
        id: id(1),
        put: PresignedPut {
            url: "https://s3.scnative.dev/x".to_owned(),
            headers: [
                ("content-length".to_owned(), "151552".to_owned()),
                (
                    "content-type".to_owned(),
                    "application/octet-stream".to_owned(),
                ),
            ]
            .into(),
        },
        thumb: Some(ReservedThumb {
            id: id(2),
            put: PresignedPut {
                url: "https://s3.scnative.dev/y".to_owned(),
                headers: Default::default(),
            },
        }),
        expires_in_s: 600,
        used: 7_340_032,
        quota: 1_073_741_824,
    };
    assert_eq!(
        serde_json::to_value(&answer).expect("json"),
        json!({
            "id": id(1),
            "put": {"url": "https://s3.scnative.dev/x", "headers": {
                "content-length": "151552", "content-type": "application/octet-stream"}},
            "thumb": {"id": id(2), "put": {"url": "https://s3.scnative.dev/y", "headers": {}}},
            "expires_in_s": 600, "used": 7_340_032, "quota": 1_073_741_824
        })
    );
    let avatar = ReserveAnswer {
        thumb: None,
        ..answer
    };
    assert!(
        serde_json::to_value(&avatar)
            .expect("json")
            .get("thumb")
            .is_none()
    );
}

#[test]
fn commit_urls_usage_mine_delete_and_restore_answers_keep_the_contract_names() {
    assert_eq!(
        serde_json::to_value(CommitAnswer {
            id: id(1),
            bytes: 176_128,
            used: 7,
            quota: 9
        })
        .expect("json"),
        json!({"id": id(1), "bytes": 176_128, "used": 7, "quota": 9})
    );
    assert_eq!(
        serde_json::to_value(UrlsAnswer {
            expires_in_s: 900,
            items: vec![
                MediaUrl::Ok {
                    id: id(1),
                    url: "https://s3.scnative.dev/a".to_owned()
                },
                MediaUrl::Deleted { id: id(2) },
                MediaUrl::Lost { id: id(3) },
            ]
        })
        .expect("json"),
        json!({"expires_in_s": 900, "items": [
            {"state": "ok", "id": id(1), "url": "https://s3.scnative.dev/a"},
            {"state": "deleted", "id": id(2)},
            {"state": "lost", "id": id(3)}
        ]})
    );
    assert_eq!(
        serde_json::to_value(UsageAnswer {
            bytes: 1,
            reserved: 2,
            quota: 3,
            files: 4,
            conversations: vec![ConversationUsage {
                conversation_id: id(5),
                bytes: 6,
                files: 7,
                gone: false
            }]
        })
        .expect("json"),
        json!({"bytes": 1, "reserved": 2, "quota": 3, "files": 4, "conversations": [
            {"conversation_id": id(5), "bytes": 6, "files": 7, "gone": false}]})
    );
    assert_eq!(
        serde_json::to_value(MinePage {
            items: vec![MineItem {
                id: id(1),
                kind: MediaKind::Avatar,
                conversation_id: id(2),
                thumb_id: None,
                bytes: 4096,
                status: MineStatus::Lost,
                chats: 3,
                created_at: 1_762_190_000_000
            }],
            next_cursor: None
        })
        .expect("json"),
        json!({"items": [{"id": id(1), "kind": "avatar", "conversation_id": id(2),
            "thumb_id": null, "bytes": 4096, "status": "lost", "chats": 3,
            "created_at": 1_762_190_000_000_i64}], "next_cursor": null})
    );
    assert_eq!(
        serde_json::to_value(DeleteAnswer {
            deleted: 3,
            freed: 528_384,
            used: 1,
            quota: 2
        })
        .expect("json"),
        json!({"deleted": 3, "freed": 528_384, "used": 1, "quota": 2})
    );
    assert_eq!(
        serde_json::to_value(RestoreAnswer {
            restored: 3,
            missed: 0,
            used: 1,
            quota: 2
        })
        .expect("json"),
        json!({"restored": 3, "missed": 0, "used": 1, "quota": 2})
    );
}

#[test]
fn urls_bodies_take_one_to_a_hundred_distinct_ids() {
    let body = |ids: Vec<Uuid>| UrlsBody {
        conversation_id: None,
        ids,
    };
    let many = |count: u128| (1..=count).map(id).collect::<Vec<_>>();
    assert!(body(many(100)).validate().is_ok());
    assert_eq!(body(many(101)).validate(), Err(ChatError::Invalid));
    assert_eq!(body(Vec::new()).validate(), Err(ChatError::Invalid));
    assert_eq!(body(vec![id(1), id(1)]).validate(), Err(ChatError::Invalid));
    let without: UrlsBody = parsed(json!({"ids": [id(1)]}));
    assert_eq!(without.conversation_id, None);
}

#[test]
fn a_selection_is_ids_or_all_and_nothing_else() {
    let picked = |value: Value| parsed::<Selection>(value).pick();
    assert_eq!(
        picked(json!({"ids": [id(1), id(1), id(2)]})),
        Ok(Picked::Ids(vec![id(1), id(2)]))
    );
    assert_eq!(picked(json!({"all": true})), Ok(Picked::All));
    assert_eq!(picked(json!({})), Err(ChatError::Invalid));
    assert_eq!(picked(json!({"ids": []})), Err(ChatError::Invalid));
    assert_eq!(
        picked(json!({"ids": [id(1)], "all": true})),
        Err(ChatError::Invalid)
    );
    let five_hundred: Vec<Uuid> = (1..=500).map(id).collect();
    assert!(picked(json!({"ids": five_hundred})).is_ok());
    let too_many: Vec<Uuid> = (1..=501).map(id).collect();
    assert_eq!(picked(json!({"ids": too_many})), Err(ChatError::Invalid));
}

#[test]
fn the_mine_query_reads_the_core_query_string() {
    let query: MineQuery = serde_urlencoded::from_str(&format!(
        "order=big&conversation_id={}&cursor=abc&limit=50",
        id(9)
    ))
    .expect("a query");
    assert_eq!(query.order, MineOrder::Big);
    assert_eq!(query.conversation_id, Some(id(9)));
    assert_eq!(query.cursor.as_deref(), Some("abc"));
    assert_eq!(query.limit, Some(50));
    let empty: MineQuery = serde_urlencoded::from_str("order=new").expect("a query");
    assert_eq!(empty, MineQuery::default());
    assert!(serde_urlencoded::from_str::<MineQuery>("order=sideways").is_err());
}

#[test]
fn the_config_view_serialises_the_core_media_config() {
    let view = MediaConfigView {
        uploads: true,
        star: true,
        quota: 1_073_741_824,
        used: 7_340_032,
        limits: crate::modules::chat::media::view::limits(),
    };
    assert_eq!(
        serde_json::to_value(view).expect("json"),
        json!({
            "uploads": true, "star": true, "quota": 1_073_741_824_u64, "used": 7_340_032,
            "limits": {
                "image_max_bytes": 614_400, "thumb_max_bytes": 32_768,
                "avatar_max_bytes": 65_536, "album_max": 10, "room_album_max": 4,
                "restore_window_s": 15
            }
        })
    );
}
