use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::common::session::SessionCtx;
use crate::error::AppResult;

use super::handlers::no_store;
use super::limits::Action;
use super::reactions::{Reply, Target};
use super::refusal::unavailable;
use super::service::MobileService;
use super::validate;

const OP: &str = "comment_likes_read";

#[derive(Deserialize)]
pub(super) struct CommentsQuery {
    comments: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommentLikes {
    pub(super) track_urn: String,
    pub(super) comments: Vec<CommentLike>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommentLike {
    pub(super) comment_urn: String,
    pub(super) liked: bool,
    pub(super) count: u64,
}

#[tracing::instrument(skip_all)]
pub(super) async fn read(
    State(mobile): State<Arc<MobileService>>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
    Query(query): Query<CommentsQuery>,
) -> AppResult<Response> {
    mobile.limits.admit(Action::Read, ctx.session_id)?;
    let likes = mobile
        .comment_likes(ctx.session_id, &ctx.sc_user_id, &track_urn, &query.comments)
        .await?;
    Ok(no_store(likes))
}

impl MobileService {
    pub(super) async fn comment_likes(
        &self,
        session_id: Uuid,
        account: &str,
        track: &str,
        comments: &str,
    ) -> AppResult<CommentLikes> {
        let track = validate::track_urn(track)?;
        let comments = validate::comment_urns(comments)?;
        let request = sc_mobile::interactions::read_comment_likes(&track, &comments);
        let reply = self.owner_reply(session_id, account, OP, &request).await?;
        let comments = likes(&reply.body, &comments).ok_or_else(unavailable)?;
        Ok(CommentLikes {
            track_urn: track,
            comments,
        })
    }
}

fn likes(body: &[u8], comments: &[String]) -> Option<Vec<CommentLike>> {
    let reply: Reply = serde_json::from_slice(body).ok()?;
    if !reply.errors.is_empty() {
        return None;
    }
    let targets = reply.data?.target_interactions.unwrap_or_default();
    Some(
        comments
            .iter()
            .map(|comment| like(comment, &targets))
            .collect(),
    )
}

fn like(comment: &str, targets: &[Target]) -> CommentLike {
    let found = targets.iter().find(|target| target.target_urn == comment);
    let count = found
        .and_then(|target| target.interaction_counts.as_ref())
        .into_iter()
        .flatten()
        .filter(|count| count.interaction_type_value_urn == sc_mobile::interactions::LIKE)
        .map(|count| count.count)
        .sum();
    CommentLike {
        comment_urn: comment.to_owned(),
        liked: found.and_then(|target| target.user_interaction.as_deref())
            == Some(sc_mobile::interactions::LIKE),
        count,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn comment(id: u32) -> String {
        format!("soundcloud:comments:{id}")
    }

    #[test]
    fn likes_are_mapped_per_requested_comment_and_missing_ones_are_empty() {
        let reply = json!({
            "data": { "targetInteractions": [
                {
                    "targetUrn": "soundcloud:comments:1",
                    "userInteraction": "sc:interactiontypevalue:like",
                    "interactionCounts": [
                        { "interactionTypeValueUrn": "sc:interactiontypevalue:like", "count": 4 }
                    ]
                },
                {
                    "targetUrn": "soundcloud:comments:2",
                    "userInteraction": null,
                    "interactionCounts": [
                        { "interactionTypeValueUrn": "sc:interactiontypevalue:like", "count": 7 },
                        { "interactionTypeValueUrn": "sc:interactiontypevalue:1f525", "count": 9 }
                    ]
                },
                { "targetUrn": "soundcloud:comments:3", "userInteraction": null, "interactionCounts": null }
            ] }
        })
        .to_string();

        let mapped = likes(
            reply.as_bytes(),
            &[comment(1), comment(2), comment(3), comment(4)],
        );

        assert_eq!(
            mapped,
            Some(vec![
                CommentLike {
                    comment_urn: comment(1),
                    liked: true,
                    count: 4
                },
                CommentLike {
                    comment_urn: comment(2),
                    liked: false,
                    count: 7
                },
                CommentLike {
                    comment_urn: comment(3),
                    liked: false,
                    count: 0
                },
                CommentLike {
                    comment_urn: comment(4),
                    liked: false,
                    count: 0
                },
            ])
        );
    }

    #[test]
    fn a_graphql_error_or_a_foreign_body_is_not_a_reading() {
        let failed = json!({ "data": null, "errors": [{ "message": "boom" }] }).to_string();
        let empty = json!({ "data": { "targetInteractions": null } }).to_string();

        assert_eq!(likes(failed.as_bytes(), &[comment(1)]), None);
        assert_eq!(likes(b"<html>", &[comment(1)]), None);
        assert_eq!(likes(b"{}", &[comment(1)]), None);
        assert_eq!(
            likes(empty.as_bytes(), &[comment(1)]),
            Some(vec![CommentLike {
                comment_urn: comment(1),
                liked: false,
                count: 0
            }])
        );
    }
}
