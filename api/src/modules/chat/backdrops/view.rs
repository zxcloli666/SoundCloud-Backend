use std::collections::HashMap;

use super::rules;
use crate::modules::chat::ChatState;
use crate::modules::chat::conversations::view::ConversationView;
use crate::modules::chat::members::roles;

pub async fn settle(chat: &ChatState, views: &mut [ConversationView]) {
    let mut stars: HashMap<String, bool> = HashMap::new();
    for view in views {
        let Some(chosen) = &view.backdrop else {
            continue;
        };
        let Ok(kind) = roles::kind(&view.kind) else {
            view.backdrop = None;
            continue;
        };
        if !rules::needs_star(kind, &chosen.backdrop) {
            continue;
        }
        let star = match stars.get(&chosen.by) {
            Some(star) => *star,
            None => {
                let star = star_of(chat, &chosen.by).await;
                stars.insert(chosen.by.clone(), star);
                star
            }
        };
        if !star {
            view.backdrop = None;
        }
    }
}

async fn star_of(chat: &ChatState, user: &str) -> bool {
    match chat.stars.is_star(user).await {
        Ok(star) => star,
        Err(error) => {
            tracing::warn!(
                code = error.code(),
                "a chat backdrop falls back while the STAR check is down"
            );
            false
        }
    }
}
