use std::sync::Arc;

use axum::Router;
use axum::middleware;
use axum::routing::{delete, get, post, put};

use crate::modules::chat::channel::{self, Channel};
use crate::modules::chat::{
    ChatState, admin, auth_bind, blocks, conversations, devices, invites, keys, members, messages,
    reports, requests, rooms, route_metrics, settings, sync, typing, users,
};

pub fn inner() -> Router<Arc<ChatState>> {
    Router::new()
        .route("/chat/config", get(channel::config_route::chat_config))
        .route("/auth/login", get(auth_bind::login))
        .route("/auth/login/status", get(auth_bind::login_status))
        .route("/auth/link/create", post(auth_bind::link_create))
        .route("/auth/link/claim", post(auth_bind::link_claim))
        .route("/auth/link/status", get(auth_bind::link_status))
        .route("/chat/devices", get(devices::list).post(devices::register))
        .route("/chat/devices/me", delete(devices::remove_me))
        .route("/chat/devices/{id}", delete(devices::remove))
        .route("/chat/devices/{id}/confirm", post(devices::confirm))
        .route("/chat/devices/{id}/not-me", post(devices::not_me))
        .route("/chat/sync", post(sync::sync))
        .route(
            "/chat/conversations",
            get(conversations::list).post(conversations::create),
        )
        .route(
            "/chat/conversations/{id}",
            get(conversations::show)
                .patch(conversations::update)
                .delete(conversations::delete),
        )
        .route("/chat/conversations/{id}/me", put(conversations::update_me))
        .route("/chat/conversations/{id}/ttl", put(conversations::set_ttl))
        .route("/chat/conversations/{id}/keys", get(keys::list))
        .route("/chat/conversations/{id}/typing", post(typing::typing))
        .route(
            "/chat/conversations/{id}/members",
            get(members::list).post(members::add),
        )
        .route(
            "/chat/conversations/{id}/members/{user}",
            delete(members::remove).put(members::update),
        )
        .route("/chat/conversations/{id}/accept", post(members::accept))
        .route("/chat/conversations/{id}/decline", post(members::decline))
        .route(
            "/chat/conversations/{id}/messages",
            get(messages::history).post(messages::send),
        )
        .route(
            "/chat/conversations/{id}/messages/{seq}",
            delete(messages::delete),
        )
        .route("/chat/conversations/{id}/invites", post(invites::create))
        .route("/chat/requests", get(requests::list))
        .route("/chat/invites/{token}", delete(invites::revoke))
        .route("/chat/invites/{token}/join", post(invites::join))
        .route("/chat/settings", get(settings::show).put(settings::update))
        .route(
            "/chat/blocks/{user}",
            put(blocks::block).delete(blocks::unblock),
        )
        .route("/chat/reports", post(reports::file))
        .route("/chat/users/{id}/card", get(users::card))
        .route("/chat/rooms/{code}/chat", get(rooms::chat))
        .route("/chat/rooms/{code}/preview", get(rooms::preview))
        .route(
            "/chat/rooms/{code}/members/{user}",
            delete(rooms::remove_member),
        )
        .route("/chat/rooms/{code}/members/{user}/mute", put(rooms::mute))
        .route(
            "/chat/rooms/{code}/moderators/{user}",
            put(rooms::moderator),
        )
        .route("/chat/rooms/{code}/settings", put(rooms::settings))
        .route("/chat/rooms/{code}/suggest", post(rooms::suggest))
        .route_layer(middleware::from_fn(route_metrics::track))
}

pub fn public<S: Clone + Send + Sync + 'static>(chat: Arc<ChatState>) -> Router<S> {
    let channel = Arc::new(Channel::new(chat.clone(), inner().with_state(chat.clone())));
    channel::router(channel).merge(admin::router(chat))
}

#[cfg(test)]
#[path = "tests/routes_tests.rs"]
mod tests;
