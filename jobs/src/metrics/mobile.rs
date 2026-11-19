use super::HANDLE;

const REQUESTS: &str = "sc_mobile_requests_total";
const ACTIONS: &str = "jobs_mobile_actions_total";
const SERVICE: &str = "jobs";

pub const ACTION_TYPES: [&str; 11] = [
    "track_reaction_set",
    "track_reaction_remove",
    "comment_delete",
    "follow_user",
    "unfollow_user",
    "like_track",
    "unlike_track",
    "comment_like",
    "comment_unlike",
    "playlist_create",
    "playlist_delete",
];
pub const OUTCOMES: [&str; 8] = [
    "written",
    "target_missing",
    "not_linked",
    "relink_required",
    "captcha",
    "failed",
    "held",
    "cooling",
];
const REQUEST_OPS: [&str; 12] = [
    "refresh",
    "track_reaction_set",
    "track_reaction_remove",
    "comment_delete",
    "follow_user",
    "unfollow_user",
    "like_track",
    "unlike_track",
    "comment_like",
    "comment_unlike",
    "playlist_create",
    "playlist_delete",
];
const STATUS_CLASSES: [&str; 10] = [
    "2xx",
    "3xx",
    "4xx",
    "5xx",
    "captcha",
    "transport",
    "too_large",
    "invalid",
    "local",
    "other",
];

pub(super) fn register() {
    for op in REQUEST_OPS {
        for class in STATUS_CLASSES {
            metrics::counter!(
                REQUESTS,
                "service" => SERVICE,
                "op" => op,
                "status_class" => class
            )
            .increment(0);
        }
    }
    for action in ACTION_TYPES {
        for outcome in OUTCOMES {
            metrics::counter!(ACTIONS, "action" => action, "outcome" => outcome).increment(0);
        }
    }
}

pub fn record_mobile_request(op: &'static str, status_class: &'static str) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(
        REQUESTS,
        "service" => SERVICE,
        "op" => op,
        "status_class" => status_class
    )
    .increment(1);
}

pub fn record_mobile_action(action: &'static str, outcome: &'static str) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(ACTIONS, "action" => action, "outcome" => outcome).increment(1);
}

#[cfg(test)]
mod tests {
    use super::super::{HANDLE, init};

    #[test]
    fn mobile_counters_are_scraped_before_the_first_linked_write() {
        init();

        let Some(handle) = HANDLE.get() else {
            return;
        };
        let rendered = handle.render();

        for op in super::REQUEST_OPS {
            assert!(
                rendered.contains(&format!(
                    "sc_mobile_requests_total{{service=\"jobs\",op=\"{op}\",status_class=\"captcha\"}} 0"
                )),
                "{op}"
            );
        }
        for action in super::ACTION_TYPES {
            for outcome in super::OUTCOMES {
                assert!(
                    rendered.contains(&format!(
                        "jobs_mobile_actions_total{{action=\"{action}\",outcome=\"{outcome}\"}} 0"
                    )),
                    "{action} {outcome}"
                );
            }
        }
    }
}
