use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::{inner, public};
use crate::modules::chat::test_support::{caller, offline_chat_state};

const CONV: &str = "0192e5c5-1a2b-7c3d-8e4f-5a6b7c8d9e0f";

fn inner_routes() -> Vec<(&'static str, String)> {
    vec![
        ("GET", "/chat/config".into()),
        ("GET", "/auth/login".into()),
        ("GET", "/auth/login/status".into()),
        ("POST", "/auth/link/create".into()),
        ("POST", "/auth/link/claim".into()),
        ("GET", "/auth/link/status".into()),
        ("POST", "/chat/devices".into()),
        ("GET", "/chat/devices".into()),
        ("DELETE", "/chat/devices/me".into()),
        ("DELETE", format!("/chat/devices/{CONV}")),
        ("POST", format!("/chat/devices/{CONV}/confirm")),
        ("POST", format!("/chat/devices/{CONV}/not-me")),
        ("POST", "/chat/sync".into()),
        ("GET", "/chat/conversations".into()),
        ("POST", "/chat/conversations".into()),
        ("GET", format!("/chat/conversations/{CONV}")),
        ("PATCH", format!("/chat/conversations/{CONV}")),
        ("DELETE", format!("/chat/conversations/{CONV}?for=me")),
        ("PUT", format!("/chat/conversations/{CONV}/me")),
        ("PUT", format!("/chat/conversations/{CONV}/ttl")),
        (
            "GET",
            format!("/chat/conversations/{CONV}/keys?from_epoch=0"),
        ),
        ("POST", format!("/chat/conversations/{CONV}/typing")),
        ("GET", format!("/chat/conversations/{CONV}/members")),
        ("POST", format!("/chat/conversations/{CONV}/members")),
        ("DELETE", format!("/chat/conversations/{CONV}/members/202")),
        ("PUT", format!("/chat/conversations/{CONV}/members/202")),
        ("POST", format!("/chat/conversations/{CONV}/accept")),
        ("POST", format!("/chat/conversations/{CONV}/decline")),
        ("POST", format!("/chat/conversations/{CONV}/messages")),
        (
            "GET",
            format!("/chat/conversations/{CONV}/messages?before=9&limit=5"),
        ),
        (
            "DELETE",
            format!("/chat/conversations/{CONV}/messages/5?for=everyone"),
        ),
        ("POST", format!("/chat/conversations/{CONV}/invites")),
        ("GET", "/chat/requests".into()),
        ("DELETE", "/chat/invites/token".into()),
        ("POST", "/chat/invites/token/join".into()),
        ("GET", "/chat/settings".into()),
        ("PUT", "/chat/settings".into()),
        ("PUT", "/chat/blocks/202".into()),
        ("DELETE", "/chat/blocks/202".into()),
        ("POST", "/chat/reports".into()),
        ("GET", "/chat/users/202/card".into()),
        ("GET", "/chat/rooms/ABC123/chat".into()),
        ("GET", "/chat/rooms/ABC123/preview".into()),
        ("DELETE", "/chat/rooms/ABC123/members/202?ban=true".into()),
        ("PUT", "/chat/rooms/ABC123/members/202/mute".into()),
        ("PUT", "/chat/rooms/ABC123/moderators/202".into()),
        ("PUT", "/chat/rooms/ABC123/settings".into()),
        ("POST", "/chat/rooms/ABC123/suggest".into()),
    ]
}

fn admin_routes() -> Vec<(&'static str, String)> {
    vec![
        ("GET", "/chat-admin/reports".into()),
        ("GET", format!("/chat-admin/reports/{CONV}")),
        ("POST", format!("/chat-admin/reports/{CONV}")),
        ("GET", format!("/chat-admin/reports/{CONV}/audit")),
        ("POST", format!("/chat-admin/reports/{CONV}/restore")),
        ("GET", "/chat-admin/restrictions".into()),
        ("POST", "/chat-admin/restrictions".into()),
        ("DELETE", format!("/chat-admin/messages/{CONV}/5")),
        ("POST", "/chat-admin/unlist/ABC123".into()),
        ("POST", "/chat-admin/rewrap".into()),
        ("GET", "/chat-admin/stats".into()),
        ("GET", "/chat-admin/metrics".into()),
        ("POST", "/chat-admin/loadtest/seed".into()),
        ("POST", "/chat-admin/loadtest/purge".into()),
    ]
}

async fn answer(
    router: &axum::Router,
    method: &str,
    uri: &str,
    with_caller: bool,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-chat-admin-token", "anything")
        .body(Body::empty())
        .expect("a request");
    if with_caller {
        request.extensions_mut().insert(caller());
    }
    let response = router.clone().oneshot(request).await.expect("an answer");
    let status = response.status();
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("a body");
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn an_inner_route_without_a_resolved_caller_is_a_server_fault_not_a_pass() {
    let router = inner().with_state(offline_chat_state(true));
    assert_eq!(
        answer(&router, "POST", "/chat/sync", false).await,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": "chat_internal"})
        )
    );
}

#[tokio::test]
async fn every_admin_route_is_closed_without_the_token_file() {
    let router: axum::Router = public(offline_chat_state(true));
    for (method, uri) in admin_routes() {
        assert_eq!(
            answer(&router, method, &uri, false).await,
            (
                StatusCode::UNAUTHORIZED,
                json!({"error": "chat_admin_unauthorized"})
            ),
            "{method} {uri}"
        );
    }
}

#[tokio::test]
async fn inner_paths_are_never_mounted_on_the_public_router() {
    let router: axum::Router = public(offline_chat_state(true));
    for (method, uri) in inner_routes() {
        let (status, _) = answer(&router, method, &uri, true).await;
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
            "{method} {uri} answered {status} in public"
        );
    }
}

#[tokio::test]
async fn a_malformed_path_id_is_a_chat_error() {
    let router = inner().with_state(offline_chat_state(true));
    assert_eq!(
        answer(&router, "GET", "/chat/conversations/not-a-uuid", true).await,
        (StatusCode::NOT_FOUND, json!({"error": "chat_not_found"}))
    );
}
