use serde_json::Value;

use crate::error::AppResult;
use crate::modules::me::MeService;

pub(super) async fn read(me: &MeService, sc_user_id: &str) -> AppResult<Value> {
    let mut profile = me.get_profile(sc_user_id).await?;
    fill_old_likes_counter(&mut profile);
    Ok(profile)
}

fn fill_old_likes_counter(profile: &mut Value) {
    let Some(profile) = profile.as_object_mut() else {
        return;
    };
    if profile
        .get("public_favorites_count")
        .is_some_and(Value::is_number)
    {
        return;
    }
    let likes = profile
        .get("likes_count")
        .filter(|count| count.is_number())
        .cloned()
        .unwrap_or_else(|| Value::from(0));
    profile.insert("public_favorites_count".into(), likes);
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
