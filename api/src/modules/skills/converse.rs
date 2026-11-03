use std::time::Duration;

use backend_contracts::remote::LinkProvider;

use super::phrases::Phrases;
use super::speech::clip;
use crate::common::admission::{AdmissionRejection, Endpoint};
use crate::modules::remote::Asked;
use crate::modules::remote::links::Grant;
use crate::state::AppState;

pub struct Said {
    pub text: String,
    pub speech: String,
}

impl Said {
    pub fn plain(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            speech: text.to_owned(),
        }
    }
}

pub async fn run(
    st: &AppState,
    grant: &Grant,
    utterance: &str,
    wait: Duration,
    phrases: &Phrases<'_>,
) -> Said {
    if utterance.trim().is_empty() {
        return Said::plain(phrases.repeat());
    }
    let identity = format!("grant:{}", grant.id);
    match st
        .admission
        .check_key(Endpoint::VoiceUtterance, &identity)
        .await
    {
        Ok(()) => {}
        Err(AdmissionRejection::Limited { .. }) => return Said::plain(phrases.too_fast()),
        Err(AdmissionRejection::Unavailable) => return Said::plain(phrases.busy()),
    }
    match st.subscriptions.is_premium(&grant.sc_user_id).await {
        Ok(true) => {}
        Ok(false) => return Said::plain(phrases.star_needed()),
        Err(error) => {
            tracing::warn!(%error, "voice skill could not read the subscription");
            return Said::plain(phrases.busy());
        }
    }
    let source = match grant.provider {
        LinkProvider::Alice => "alice",
        LinkProvider::Salut => "salut",
    };
    match st
        .remote
        .dispatch(&grant.sc_user_id, source, utterance, wait)
        .await
    {
        Asked::Offline | Asked::Silent => Said::plain(phrases.offline()),
        Asked::Reply(reply) => {
            let display = clip(&reply.display);
            let speech = clip(&reply.speech);
            let fallback = if reply.ok {
                phrases.done()
            } else {
                phrases.failed()
            };
            let text = [&display, &speech]
                .into_iter()
                .find(|part| !part.is_empty())
                .cloned()
                .unwrap_or_else(|| fallback.to_owned());
            let speech = if speech.is_empty() {
                text.clone()
            } else {
                speech
            };
            Said { text, speech }
        }
    }
}
