use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use wreq::Method;

use super::client::{SoundCloudClient, SoundCloudError};

#[derive(Clone, Copy, Debug)]
pub enum WriteHold {
    App(i64),
    Account(i64),
}

impl WriteHold {
    pub fn seconds(self) -> i64 {
        match self {
            Self::App(seconds) | Self::Account(seconds) => seconds,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::App(_) => "SoundCloud paused this action for the application",
            Self::Account(_) => "SoundCloud limits this action for the account",
        }
    }
}

pub struct WriteGate<'a> {
    check: Option<&'a (dyn Fn() -> Option<WriteHold> + Sync)>,
    passed: AtomicBool,
}

impl<'a> WriteGate<'a> {
    #[cfg(test)]
    pub fn open() -> Self {
        Self {
            check: None,
            passed: AtomicBool::new(true),
        }
    }

    pub fn before_first_write(check: &'a (dyn Fn() -> Option<WriteHold> + Sync)) -> Self {
        Self {
            check: Some(check),
            passed: AtomicBool::new(false),
        }
    }

    pub fn check(&self) -> Result<(), SoundCloudError> {
        if self.passed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        match self.check.and_then(|check| check()) {
            Some(hold) => Err(SoundCloudError::Paused(hold)),
            None => Ok(()),
        }
    }
}

pub struct GatedClient<'a> {
    client: &'a SoundCloudClient,
    gate: &'a WriteGate<'a>,
}

impl<'a> GatedClient<'a> {
    pub fn new(client: &'a SoundCloudClient, gate: &'a WriteGate<'a>) -> Self {
        Self { client, gate }
    }

    pub async fn post(
        &self,
        path: &str,
        access_token: &str,
        body: Option<&Value>,
    ) -> Result<Value, SoundCloudError> {
        self.client
            .send_gated(Method::POST, path, access_token, body, self.gate)
            .await
    }

    pub async fn put(
        &self,
        path: &str,
        access_token: &str,
        body: Option<&Value>,
    ) -> Result<Value, SoundCloudError> {
        self.client
            .send_gated(Method::PUT, path, access_token, body, self.gate)
            .await
    }

    pub async fn delete(&self, path: &str, access_token: &str) -> Result<Value, SoundCloudError> {
        self.client
            .send_gated(Method::DELETE, path, access_token, None, self.gate)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    #[test]
    fn only_the_first_write_is_checked() {
        let calls = AtomicUsize::new(0);
        let check = || (calls.fetch_add(1, Ordering::SeqCst) > 0).then_some(WriteHold::App(60));
        let gate = WriteGate::before_first_write(&check);

        assert!(gate.check().is_ok());
        assert!(gate.check().is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_hold_stops_the_first_write() {
        let check = || Some(WriteHold::Account(30));
        let gate = WriteGate::before_first_write(&check);

        assert!(matches!(
            gate.check(),
            Err(SoundCloudError::Paused(WriteHold::Account(30)))
        ));
    }
}
