use sc_mobile::{Caller, MobileClient, TokenSet, me};
use serde_json::Value;

use crate::journal::{Journal, Sent, failure_note, json_of};
use crate::sign_in::{Session, refusal, remember, token_notes, view};

pub async fn read_me(
    client: &MobileClient,
    caller: &Caller,
    journal: &mut Journal,
    step: &str,
) -> Option<String> {
    let request = me::me();
    let outcome = client.call(caller, &request).await;
    let urn = json_of(&outcome).as_ref().and_then(user_urn);
    let note = match (&urn, failure_note(&outcome)) {
        (_, Some(failure)) => failure,
        (Some(urn), None) => format!("signed in as {urn}"),
        (None, None) => "the answer has no user urn".to_owned(),
    };
    journal.reply(step, Sent::of(&request), &outcome, vec![note]);
    urn
}

pub async fn refresh(
    client: &MobileClient,
    session: &Session,
    journal: &mut Journal,
) -> Option<TokenSet> {
    let Some(refresh_token) = &session.tokens.refresh_token else {
        journal.skip("refresh", "the code exchange gave no refresh_token");
        return None;
    };
    match client.refresh(refresh_token, &session.device_id).await {
        Ok(tokens) => {
            remember(journal, &tokens);
            let rotated = match &tokens.refresh_token {
                Some(new) if new.expose() == refresh_token.expose() => "same refresh_token back",
                Some(_) => "refresh_token rotated",
                None => "no refresh_token back",
            };
            let mut notes = vec!["refresh works".to_owned(), rotated.to_owned()];
            notes.extend(token_notes(&tokens));
            journal.event("refresh", true, view(&tokens), notes);
            Some(tokens)
        }
        Err(error) => {
            journal.event("refresh", false, refusal(&error), vec![error.to_string()]);
            None
        }
    }
}

fn user_urn(body: &Value) -> Option<String> {
    let user = body.get("user").unwrap_or(body);
    if let Some(urn) = user.get("urn").and_then(Value::as_str) {
        return Some(urn.to_owned());
    }
    user.get("id")
        .and_then(Value::as_u64)
        .map(|id| format!("soundcloud:users:{id}"))
}
