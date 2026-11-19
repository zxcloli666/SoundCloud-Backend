use redact::Secret;
use sc_mobile::{Caller, MobileClient, Pkce, TokenSet, authorize_url, new_device_id, new_state};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use url::form_urlencoded;

use crate::journal::Journal;
use crate::scrub::REDACTED;

pub struct Session {
    pub device_id: String,
    pub tokens: TokenSet,
}

struct Redirect {
    code: Secret<String>,
    state_returned: bool,
}

impl Session {
    pub fn caller(&self) -> Caller {
        Caller::user(&self.device_id, self.tokens.access_token.clone())
    }
}

pub async fn sign_in(client: &MobileClient, journal: &mut Journal) -> Option<Session> {
    let (pkce, state) = match (Pkce::generate(), new_state()) {
        (Ok(pkce), Ok(state)) => (pkce, state),
        _ => {
            journal.event(
                "authorize",
                false,
                Value::Null,
                vec!["the random source failed".to_owned()],
            );
            return None;
        }
    };
    let device_id = new_device_id();
    journal.hide(pkce.verifier().expose());
    journal.hide(state.expose());
    journal.hide(&device_id);

    let url = match authorize_url(client.app(), pkce.challenge(), state.expose(), &device_id) {
        Ok(url) => url,
        Err(error) => {
            journal.event("authorize", false, Value::Null, vec![error.to_string()]);
            return None;
        }
    };
    eprintln!(
        "\nOpen this address in a browser, sign in with the test account and approve access:\n\n{url}\n\n\
         The browser then tries to open sc://auth?code=... and fails. Copy that whole address \
         (or only the code) from the address bar or the developer tools, paste it here and press Enter:"
    );
    journal.event(
        "authorize",
        true,
        json!({ "url": url.as_str() }),
        vec!["authorize URL printed".to_owned()],
    );

    let pasted = read_line().await;
    let redirect = match parse_redirect(pasted.expose(), state.expose()) {
        Ok(redirect) => redirect,
        Err(why) => {
            journal.event("redirect", false, Value::Null, vec![why]);
            return None;
        }
    };
    journal.hide(redirect.code.expose());
    let state_note = if redirect.state_returned {
        "state returned and matches"
    } else {
        "state not returned"
    };
    journal.event("redirect", true, Value::Null, vec![state_note.to_owned()]);

    match client
        .exchange_code(&redirect.code, pkce.verifier(), &device_id)
        .await
    {
        Ok(tokens) => {
            remember(journal, &tokens);
            journal.event("exchange_code", true, view(&tokens), token_notes(&tokens));
            Some(Session { device_id, tokens })
        }
        Err(error) => {
            journal.event(
                "exchange_code",
                false,
                refusal(&error),
                vec![error.to_string()],
            );
            None
        }
    }
}

pub fn remember(journal: &mut Journal, tokens: &TokenSet) {
    journal.hide(tokens.access_token.expose());
    if let Some(refresh) = &tokens.refresh_token {
        journal.hide(refresh.expose());
    }
}

pub fn view(tokens: &TokenSet) -> Value {
    json!({
        "access_token": REDACTED,
        "refresh_token": tokens.refresh_token.as_ref().map(|_| REDACTED),
        "expires_in": tokens.expires_in,
        "scope": tokens.scope,
    })
}

pub fn token_notes(tokens: &TokenSet) -> Vec<String> {
    vec![
        if tokens.refresh_token.is_some() {
            "refresh_token present".to_owned()
        } else {
            "no refresh_token".to_owned()
        },
        tokens
            .expires_in
            .map_or("no expires_in".to_owned(), |seconds| {
                format!("expires_in {seconds}")
            }),
        tokens
            .scope
            .as_ref()
            .map_or("no scope".to_owned(), |scope| format!("scope {scope}")),
    ]
}

pub fn refusal(error: &sc_mobile::MobileError) -> Value {
    match error {
        sc_mobile::MobileError::Refused {
            status,
            error,
            description,
        } => json!({ "status": status, "error": error, "error_description": description }),
        other => json!({ "error": other.to_string() }),
    }
}

async fn read_line() -> Secret<String> {
    let mut line = String::new();
    let read = BufReader::new(tokio::io::stdin())
        .read_line(&mut line)
        .await;
    if read.is_err() {
        line.clear();
    }
    Secret::new(line.trim().to_owned())
}

fn parse_redirect(pasted: &str, expected_state: &str) -> Result<Redirect, String> {
    if pasted.is_empty() {
        return Err("nothing was pasted".to_owned());
    }
    if !pasted.contains("code=") && !pasted.contains("error=") {
        return Ok(Redirect {
            code: Secret::new(pasted.to_owned()),
            state_returned: false,
        });
    }
    let query = pasted
        .split_once('?')
        .map_or(pasted, |(_, query)| query)
        .split('#')
        .next()
        .unwrap_or_default();
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (name, value) in form_urlencoded::parse(query.as_bytes()) {
        match name.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }
    if let Some(error) = error {
        return Err(format!("the sign-in page returned error {error}"));
    }
    if state
        .as_deref()
        .is_some_and(|state| state != expected_state)
    {
        return Err("the returned state does not match this sign-in".to_owned());
    }
    let code = code
        .filter(|code| !code.is_empty())
        .ok_or_else(|| "the pasted address has no code".to_owned())?;
    Ok(Redirect {
        code: Secret::new(code),
        state_returned: state.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_full_redirect_gives_the_code_and_checks_the_state() {
        let Ok(redirect) = parse_redirect("sc://auth?code=abc%2Fd&state=s1", "s1") else {
            panic!("a matching redirect is refused");
        };
        assert_eq!(redirect.code.expose(), "abc/d");
        assert!(redirect.state_returned);

        assert!(parse_redirect("sc://auth?code=abc&state=other", "s1").is_err());
        assert!(parse_redirect("sc://auth?error=access_denied", "s1").is_err());
        assert!(parse_redirect("sc://auth?state=s1&code=", "s1").is_err());
        assert!(parse_redirect("", "s1").is_err());
    }

    #[test]
    fn a_bare_code_or_a_redirect_without_state_is_taken_as_is() {
        let bare = parse_redirect("abc", "s1");
        assert!(
            bare.is_ok_and(|redirect| redirect.code.expose() == "abc" && !redirect.state_returned)
        );

        let stateless = parse_redirect("sc://auth?code=abc", "s1");
        assert!(stateless.is_ok_and(|redirect| !redirect.state_returned));
    }
}
