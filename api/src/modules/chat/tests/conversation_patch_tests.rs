use serde_json::json;
use sqlx::PgPool;

use super::*;
use crate::modules::chat::conversations::fixtures::{access_of, dm, group};
use crate::modules::chat::model::MemberState;

fn settings(admins_only: Option<bool>, history_visible: Option<bool>) -> ConversationPatch {
    ConversationPatch {
        settings: Some(SettingsPatch {
            admins_only,
            history_visible,
        }),
        ..ConversationPatch::default()
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn only_group_admins_patch_a_chat(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[("202", MemberState::Active)]).await;
    let member = access_of(&pool, id, "202").await;
    assert_eq!(
        apply(&pool, &member, settings(Some(true), None))
            .await
            .err(),
        Some(ChatError::NotMember)
    );
    let direct = dm(&pool, "101", "303", MemberState::Active).await;
    let peer = access_of(&pool, direct, "101").await;
    assert_eq!(
        apply(&pool, &peer, settings(Some(true), None)).await.err(),
        Some(ChatError::NotMember)
    );
    let owner = access_of(&pool, id, "101").await;
    assert_eq!(
        apply(&pool, &owner, ConversationPatch::default())
            .await
            .err(),
        Some(ChatError::Invalid)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn settings_merge_and_a_rename_leaves_a_row(pool: PgPool) -> ChatResult<()> {
    let id = group(&pool, "101", &[("202", MemberState::Active)]).await;
    let owner = access_of(&pool, id, "101").await;
    let quiet = apply(&pool, &owner, settings(Some(true), None)).await?;
    assert!(quiet.is_none());
    let hidden = apply(&pool, &owner, settings(None, Some(false))).await?;
    assert!(hidden.is_none());
    let merged = access_of(&pool, id, "202").await.settings;
    assert!(merged.admins_only);
    assert!(!merged.history_visible);
    let renamed = ConversationPatch {
        meta_ct: Some(vec![7; 64]),
        meta_epoch: Some(0),
        settings: None,
    };
    let row = apply(&pool, &owner, renamed).await?.expect("a renamed row");
    let body: serde_json::Value =
        serde_json::from_slice(row.body.as_deref().unwrap_or_default()).expect("system json");
    assert_eq!(body, json!({"kind": "renamed", "actor": "101"}));
    let stored: (Option<Vec<u8>>, Option<i32>) =
        sqlx::query_as("SELECT meta_ct, meta_epoch FROM chat.conversations WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored, (Some(vec![7; 64]), Some(0)));
    Ok(())
}

#[test]
fn a_mute_is_a_positive_time_or_none() {
    assert_eq!(mute_until(None), Ok(None));
    assert_eq!(mute_until(Some(0)), Err(ChatError::Invalid));
    assert_eq!(mute_until(Some(-5)), Err(ChatError::Invalid));
    let forever = mute_until(Some(i64::MAX)).expect("a capped mute");
    assert_eq!(
        forever.map(|at| at.timestamp_millis()),
        Some(MUTE_FOREVER_MS)
    );
}

#[test]
fn unknown_settings_are_refused() {
    let parsed: Result<ConversationPatch, _> =
        serde_json::from_value(json!({"settings": {"slow_mode_s": 5}}));
    assert!(parsed.is_err());
}
