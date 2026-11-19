use sc_mobile::{Kind, MobileReply, MobileRequest};
use serde_json::Value;
use wreq::StatusCode;

use crate::handlers::sync_queue::client::{SoundCloudError, api_error};

pub(in crate::handlers::sync_queue) fn judge(
    request: &MobileRequest,
    reply: &MobileReply,
) -> Result<Value, SoundCloudError> {
    if !reply.is_success() {
        let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::BAD_GATEWAY);
        return Err(api_error(status, reply.body.to_vec(), None));
    }
    let body = if reply.body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&reply.body).unwrap_or(Value::Null)
    };
    if request.kind() == Kind::Mutation && graph_refused(&body) {
        return Err(SoundCloudError::Mobile(
            "the graph answered with errors".to_owned(),
        ));
    }
    Ok(body)
}

fn graph_refused(body: &Value) -> bool {
    body.get("errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| !errors.is_empty())
}
