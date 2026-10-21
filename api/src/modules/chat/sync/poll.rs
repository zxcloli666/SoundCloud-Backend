use tokio::time::{Instant, sleep, sleep_until};
use uuid::Uuid;

use crate::modules::chat::hub::{Focus, Parked};
use crate::modules::chat::model::Tier;
use crate::modules::chat::sync::answer::{self, Ask};
use crate::modules::chat::sync::bucket::OVER_BUDGET_WAIT;
use crate::modules::chat::sync::changes::{self, Changes, Versions};
use crate::modules::chat::sync::incarnation;
use crate::modules::chat::sync::plan::Plan;
use crate::modules::chat::sync::reply::SyncReply;
use crate::modules::chat::sync::wire::SyncRequest;
use crate::modules::chat::{ChatCaller, ChatResult, ChatState, devices};

const ANSWERS: &str = "chat_poll_answers_total";
const PARKED: &str = "chat_parked_polls";
const SYNC_SECONDS: &str = "chat_sync_seconds";
const WAKE_LATENCY: &str = "chat_wake_latency_seconds";

enum Look {
    Reset(Versions),
    News(Changes),
    Quiet,
}

struct Poll<'a> {
    chat: &'a ChatState,
    caller: &'a ChatCaller,
    device: Uuid,
    request: &'a SyncRequest,
    plan: Plan,
    incarnation: Uuid,
}

pub async fn run(
    chat: &ChatState,
    caller: &ChatCaller,
    device: Uuid,
    request: &SyncRequest,
) -> ChatResult<SyncReply> {
    let poll = Poll {
        chat,
        caller,
        device,
        request,
        plan: Plan::new(&chat.config, caller, request),
        incarnation: incarnation::current(chat).await?,
    };
    let reply = poll.answer().await?;
    metrics::counter!(ANSWERS, "reason" => reply.reason()).increment(1);
    Ok(reply)
}

pub fn needs_reset(request: &SyncRequest, incarnation: Uuid, versions: &Versions) -> bool {
    versions.fresh
        || request.inc != Some(incarnation)
        || request.pos < versions.floor
        || request.pos > versions.version
}

impl Poll<'_> {
    fn user_id(&self) -> &str {
        &self.caller.user_id
    }

    async fn answer(&self) -> ChatResult<SyncReply> {
        let hub = &self.chat.hub;
        let checked = hub.focus_of(self.user_id(), self.device);
        let new_focus = self.request.focus.is_some()
            && self.request.focus != checked.map(|focus| focus.conversation);
        if self.request.focus.is_none() && checked.is_some() {
            hub.set_focus(self.user_id(), self.device, None);
        }
        let parked_focus = checked
            .filter(|focus| focus.live && !new_focus)
            .map(|focus| focus.conversation);
        let mut parked = hub.park(self.user_id(), self.device, parked_focus);
        if self.chat.shutdown.is_cancelled() {
            return Ok(SyncReply::later());
        }
        let look = self.look(new_focus).await?;
        if !matches!(look, Look::Quiet) || self.plan.hold.is_zero() {
            return self.immediate(&parked, look, new_focus).await;
        }
        if hub.parked() > self.chat.config.max_parked {
            return Ok(SyncReply::later());
        }
        self.hold(&mut parked).await
    }

    async fn immediate(
        &self,
        parked: &Parked<'_>,
        look: Look,
        include_focus: bool,
    ) -> ChatResult<SyncReply> {
        if self.chat.hub.take_token(self.user_id(), self.device) {
            return self.reply(look, include_focus, None).await;
        }
        tokio::select! {
            () = self.chat.shutdown.cancelled() => return Ok(SyncReply::later()),
            () = sleep(OVER_BUDGET_WAIT) => {}
        }
        if parked.superseded() {
            return Ok(SyncReply::Superseded);
        }
        let look = self.look(include_focus).await?;
        self.reply(look, include_focus, None).await
    }

    async fn hold(&self, parked: &mut Parked<'_>) -> ChatResult<SyncReply> {
        let _gauge = ParkedGauge::new(self.caller.tier);
        let deadline = Instant::now() + self.plan.hold;
        loop {
            let signal = tokio::select! {
                () = self.chat.shutdown.cancelled() => return Ok(SyncReply::later()),
                () = sleep_until(deadline) => return Ok(SyncReply::Idle),
                signal = parked.next() => signal,
            };
            if signal.superseded {
                return Ok(SyncReply::Superseded);
            }
            if let Some(at) = signal.at {
                tokio::select! {
                    () = self.chat.shutdown.cancelled() => return Ok(SyncReply::later()),
                    () = sleep_until(at) => {}
                }
            }
            if parked.superseded() {
                return Ok(SyncReply::Superseded);
            }
            let look = self.look(signal.focus).await?;
            if !matches!(look, Look::Quiet) {
                devices::require(self.chat, self.caller).await?;
                return self.reply(look, signal.focus, signal.woken).await;
            }
        }
    }

    async fn look(&self, include_focus: bool) -> ChatResult<Look> {
        let changes = changes::read(self.chat, self.user_id(), self.request.pos).await?;
        if needs_reset(self.request, self.incarnation, &changes.versions) {
            return Ok(Look::Reset(changes.versions));
        }
        if include_focus || changes.versions.version > self.request.pos {
            return Ok(Look::News(changes));
        }
        Ok(Look::Quiet)
    }

    async fn reply(
        &self,
        look: Look,
        include_focus: bool,
        woken: Option<Instant>,
    ) -> ChatResult<SyncReply> {
        let started = Instant::now();
        let reply = match look {
            Look::Quiet => SyncReply::Idle,
            Look::Reset(versions) => SyncReply::Answer(Box::new(
                answer::reset(
                    self.chat,
                    self.user_id(),
                    self.device,
                    &versions,
                    self.incarnation,
                )
                .await?,
            )),
            Look::News(changes) => {
                let ask = Ask {
                    user_id: self.user_id(),
                    device: self.device,
                    known: &self.request.known,
                    focus: self.request.focus,
                    include_focus,
                    budget: self.plan.budget,
                };
                let built = answer::build(self.chat, &ask, &changes, self.incarnation).await?;
                if let Some(live) = built.focus_live {
                    self.chat.hub.set_focus(
                        self.user_id(),
                        self.device,
                        self.request
                            .focus
                            .map(|conversation| Focus { conversation, live }),
                    );
                }
                SyncReply::Answer(Box::new(built.answer))
            }
        };
        metrics::histogram!(SYNC_SECONDS).record(started.elapsed().as_secs_f64());
        if let Some(woken) = woken {
            metrics::histogram!(WAKE_LATENCY).record(woken.elapsed().as_secs_f64());
        }
        Ok(reply)
    }
}

struct ParkedGauge {
    tier: &'static str,
}

impl ParkedGauge {
    fn new(tier: Tier) -> Self {
        let tier = match tier {
            Tier::Pro => "pro",
            Tier::Relay => "relay",
            Tier::Direct => "direct",
            Tier::Unspecified => "unspecified",
        };
        metrics::gauge!(PARKED, "tier" => tier).increment(1.0);
        Self { tier }
    }
}

impl Drop for ParkedGauge {
    fn drop(&mut self) {
        metrics::gauge!(PARKED, "tier" => self.tier).decrement(1.0);
    }
}
