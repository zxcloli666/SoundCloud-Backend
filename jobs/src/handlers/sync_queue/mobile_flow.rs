use backend_contracts::SYNC_QUEUE_MAX_RETRIES;

use super::actions;
use super::mobile::{self, Absence, Failure, Opened, Refusal, Route};
use super::model::ClaimedMutation;
use super::write_gate::WriteGate;
use super::{PAUSED_ACTION_RECHECK_SECONDS, SyncQueueHandler, account_action};

const BOT_CHECK_COOLDOWN_SECONDS: i64 = 60 * 60;
use crate::metrics::record_mobile_action;

pub(super) enum Flow {
    Done,
    Apiv1,
}

impl SyncQueueHandler {
    pub(super) async fn send_mobile(
        &self,
        mutation: &ClaimedMutation,
        route: Route,
    ) -> Result<Flow, anyhow::Error> {
        let action = metric_action(&mutation.action_type);
        let request = match mobile::build(mutation) {
            Ok(request) => request,
            Err(_) if route == Route::MobileThenApiv1 => return Ok(Flow::Apiv1),
            Err(error) => {
                self.record_action_failure(mutation, &error).await?;
                return Ok(Flow::Done);
            }
        };
        if self
            .bot_checks
            .remaining_seconds(&account_action(mutation))
            .is_some()
        {
            record_mobile_action(action, "cooling");
            return self.refuse(mutation, route, "mobile_bot_check").await;
        }
        let mut linked = match self.mobile.open(&mutation.user_id).await? {
            Opened::Ready(linked) => linked,
            Opened::Absent(absence) => {
                return self.without_mobile(mutation, route, action, absence).await;
            }
            Opened::Later(seconds) => {
                record_mobile_action(action, "held");
                self.postpone_before_post(
                    mutation,
                    "the mobile token is not ready",
                    seconds.min(PAUSED_ACTION_RECHECK_SECONDS),
                )
                .await?;
                return Ok(Flow::Done);
            }
        };
        if mutation.action_type == "playlist_create"
            && mutation.has_ambiguous_remote_attempt()
            && !self.confirmed_not_created(mutation).await?
        {
            return Ok(Flow::Done);
        }
        let oauth_app_id = Some(linked.oauth_app_id());
        let Some(probing) = self.admit(mutation, oauth_app_id).await? else {
            record_mobile_action(action, "held");
            return Ok(Flow::Done);
        };
        if !self.repository.record_remote_attempt(mutation).await? {
            self.repository.release(mutation).await?;
            return Ok(Flow::Done);
        }
        let hold = || self.hold_before_write(mutation, oauth_app_id, probing);
        let gate = WriteGate::before_first_write(&hold);
        match self.mobile.send(&mut linked, action, &request, &gate).await {
            Ok(reply) => {
                let written = mobile::judge(&request, &reply);
                let remote = actions::written_or_closed(mutation, written)
                    .and_then(|remote| mobile::shape(mutation, remote));
                match remote {
                    Ok(remote) => {
                        let outcome = match remote {
                            actions::Remote::Written(_) => "written",
                            actions::Remote::TargetMissing => "target_missing",
                        };
                        record_mobile_action(action, outcome);
                        self.finish(mutation, remote, oauth_app_id).await?;
                    }
                    Err(error) => {
                        record_mobile_action(action, "failed");
                        self.pause_writes_when_refused(mutation, &error, oauth_app_id)
                            .await;
                        self.record_action_failure(mutation, &error).await?;
                    }
                }
                Ok(Flow::Done)
            }
            Err(Failure::Held(hold)) => {
                record_mobile_action(action, "held");
                self.postpone_unsent(mutation, hold, oauth_app_id).await?;
                Ok(Flow::Done)
            }
            Err(Failure::Later(seconds)) => {
                record_mobile_action(action, "held");
                self.postpone_before_post(
                    mutation,
                    "the mobile token is not ready",
                    seconds.min(PAUSED_ACTION_RECHECK_SECONDS),
                )
                .await?;
                Ok(Flow::Done)
            }
            Err(Failure::Refused(refusal)) => {
                let (outcome, reason) = match refusal {
                    Refusal::Captcha => {
                        self.bot_checks
                            .pause(account_action(mutation), BOT_CHECK_COOLDOWN_SECONDS);
                        ("captcha", "mobile_bot_check")
                    }
                    Refusal::RelinkRequired => ("relink_required", "mobile_relink_required"),
                };
                record_mobile_action(action, outcome);
                self.refuse(mutation, route, reason).await
            }
            Err(Failure::Error(error)) => {
                record_mobile_action(action, "failed");
                self.pause_writes_when_refused(mutation, &error, oauth_app_id)
                    .await;
                self.record_action_failure(mutation, &error).await?;
                Ok(Flow::Done)
            }
        }
    }

    async fn confirmed_not_created(
        &self,
        mutation: &ClaimedMutation,
    ) -> Result<bool, anyhow::Error> {
        let token = match self
            .connections
            .access_token(&self.token_client, &mutation.user_id)
            .await
        {
            Ok(token) => token,
            Err(error) => {
                self.record_connection_failure(mutation, &error).await?;
                return Ok(false);
            }
        };
        Ok(self
            .confirm_unclear_create(mutation, token)
            .await?
            .is_some())
    }

    async fn without_mobile(
        &self,
        mutation: &ClaimedMutation,
        route: Route,
        action: &'static str,
        absence: Absence,
    ) -> Result<Flow, anyhow::Error> {
        let (outcome, reason) = match absence {
            Absence::NotLinked => ("not_linked", "mobile_not_linked"),
            Absence::RelinkRequired => ("relink_required", "mobile_relink_required"),
        };
        if route == Route::MobileThenApiv1 && matches!(absence, Absence::NotLinked) {
            return Ok(Flow::Apiv1);
        }
        record_mobile_action(action, outcome);
        self.refuse(mutation, route, reason).await
    }

    async fn refuse(
        &self,
        mutation: &ClaimedMutation,
        route: Route,
        reason: &str,
    ) -> Result<Flow, anyhow::Error> {
        if route == Route::MobileThenApiv1 {
            return Ok(Flow::Apiv1);
        }
        self.repository
            .park(mutation, SYNC_QUEUE_MAX_RETRIES, reason)
            .await?;
        Ok(Flow::Done)
    }
}

fn metric_action(action_type: &str) -> &'static str {
    crate::metrics::MOBILE_ACTION_TYPES
        .iter()
        .copied()
        .find(|known| *known == action_type)
        .unwrap_or("other")
}
