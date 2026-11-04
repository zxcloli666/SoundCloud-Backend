use axum::extract::State;
use sqlx::PgPool;

use super::{DmPolicy, SettingsPatch, UserSettings, load, save, show, update};
use crate::modules::chat::ChatError;
use crate::modules::chat::body::ChatJson;
use crate::modules::chat::moderation::testing::{state, state_with};
use crate::modules::chat::sync::prefs::Prefs;
use crate::modules::chat::test_support::caller;

#[sqlx::test(migrations = "./migrations")]
async fn settings_default_until_saved_and_save_only_what_was_sent(pool: PgPool) {
    let chat = state(pool, None);
    assert_eq!(
        load(&chat, "101").await.expect("read"),
        UserSettings::default()
    );
    assert_eq!(UserSettings::default().dm_policy, DmPolicy::Following);

    let saved = save(
        &chat,
        "101",
        &SettingsPatch {
            dm_policy: Some(DmPolicy::Everyone),
            typing: Some(false),
            ..SettingsPatch::default()
        },
    )
    .await
    .expect("saved");
    assert_eq!(saved.dm_policy, DmPolicy::Everyone);
    assert!(!saved.typing);
    assert!(saved.read_receipts);

    let again = save(
        &chat,
        "101",
        &SettingsPatch {
            presence_share: Some(true),
            ..SettingsPatch::default()
        },
    )
    .await
    .expect("saved");
    assert_eq!(
        again,
        UserSettings {
            dm_policy: DmPolicy::Everyone,
            read_receipts: true,
            typing: false,
            presence_share: true,
        }
    );
    assert_eq!(load(&chat, "101").await.expect("read"), again);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_settings_page_lists_blocks_and_refuses_an_empty_change(pool: PgPool) {
    let chat = state(pool.clone(), None);
    sqlx::query(
        "INSERT INTO chat.blocks (user_id, blocked_user) VALUES ('101', '7'), ('101', '8')",
    )
    .execute(&pool)
    .await
    .expect("blocks");
    let shown = show(caller(), State(chat.clone()))
        .await
        .expect("settings")
        .0;
    assert_eq!(shown.blocked.len(), 2);
    assert_eq!(shown.restricted_until, None);
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM chat.users WHERE user_id = '101')"
        )
        .fetch_one(&pool)
        .await
        .expect("read")
    );
    assert_eq!(
        update(
            caller(),
            State(chat.clone()),
            ChatJson(SettingsPatch::default())
        )
        .await
        .err(),
        Some(ChatError::Invalid)
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_saved_change_reaches_the_sync_prefs_at_once(pool: PgPool) {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    let redis = crate::redis::connect_url(&url).expect("a redis pool");
    let chat = state_with(pool, redis, None);
    chat.hub.prefs.remember("101", Prefs::default());

    let changed = update(
        caller(),
        State(chat.clone()),
        ChatJson(SettingsPatch {
            dm_policy: Some(DmPolicy::Nobody),
            read_receipts: Some(false),
            presence_share: Some(true),
            ..SettingsPatch::default()
        }),
    )
    .await
    .expect("updated")
    .0;
    assert_eq!(changed.dm_policy, DmPolicy::Nobody);
    assert_eq!(
        chat.hub.prefs.known("101"),
        Some(Prefs {
            read_receipts: false,
            typing: true,
            presence_share: true,
        })
    );
    assert_eq!(chat.hub.prefs.sharing("101"), Some(true));
}

#[test]
fn unknown_settings_fields_are_refused() {
    assert!(serde_json::from_str::<SettingsPatch>(r#"{"dm_policy":"everyone"}"#).is_ok());
    assert!(serde_json::from_str::<SettingsPatch>(r#"{"dm_policy":"friends"}"#).is_err());
    assert!(serde_json::from_str::<SettingsPatch>(r#"{"theme":"dark"}"#).is_err());
}
