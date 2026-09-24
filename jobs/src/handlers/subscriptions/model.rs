use std::collections::HashSet;
use std::io::{self, Write};

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

const MAX_USER_URN_BYTES: usize = 1_024;

#[derive(Debug, Deserialize, FromRow, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Subscription {
    pub user_urn: String,
    pub exp_date: i64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub(super) struct SubscriptionSnapshot(Vec<Subscription>);

impl SubscriptionSnapshot {
    pub fn decode(bytes: &[u8], max_entries: usize) -> Result<Self, SnapshotError> {
        let snapshot = serde_json::from_slice::<Self>(bytes).map_err(SnapshotError::Decode)?;
        snapshot.validate(max_entries)?;
        Ok(snapshot)
    }

    pub fn from_entries(
        entries: Vec<Subscription>,
        max_entries: usize,
    ) -> Result<Self, SnapshotError> {
        let snapshot = Self(entries);
        snapshot.validate(max_entries)?;
        Ok(snapshot)
    }

    pub fn encode(&self, max_bytes: usize) -> Result<Vec<u8>, SnapshotError> {
        let mut output = LimitedBuffer::new(max_bytes);
        let result = serde_json::to_writer(&mut output, self);
        if output.exceeded_limit() {
            return Err(SnapshotError::TooLarge { max_bytes });
        }
        result.map_err(SnapshotError::Encode)?;
        Ok(output.into_bytes())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn into_columns(self) -> (Vec<String>, Vec<i64>) {
        self.0
            .into_iter()
            .map(|subscription| (subscription.user_urn, subscription.exp_date))
            .unzip()
    }

    fn validate(&self, max_entries: usize) -> Result<(), SnapshotError> {
        if self.0.len() > max_entries {
            return Err(SnapshotError::TooManyEntries { max_entries });
        }

        let mut user_urns = HashSet::with_capacity(self.0.len());
        for (index, subscription) in self.0.iter().enumerate() {
            let user_urn_bytes = subscription.user_urn.len();
            if user_urn_bytes == 0 || user_urn_bytes > MAX_USER_URN_BYTES {
                return Err(SnapshotError::InvalidUserUrn { index });
            }
            if !user_urns.insert(subscription.user_urn.as_str()) {
                return Err(SnapshotError::DuplicateUserUrn { index });
            }
        }

        Ok(())
    }
}

struct LimitedBuffer {
    bytes: Vec<u8>,
    max_bytes: usize,
    exceeded_limit: bool,
}

impl LimitedBuffer {
    fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(max_bytes.min(8 * 1_024)),
            max_bytes,
            exceeded_limit: false,
        }
    }

    fn exceeded_limit(&self) -> bool {
        self.exceeded_limit
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let remaining = self.max_bytes.saturating_sub(self.bytes.len());
        if bytes.len() > remaining {
            self.exceeded_limit = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "snapshot exceeded its configured size limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum SnapshotError {
    #[error("snapshot JSON is invalid: {0}")]
    Decode(serde_json::Error),

    #[error("snapshot JSON encoding failed: {0}")]
    Encode(serde_json::Error),

    #[error("snapshot contains more than {max_entries} subscriptions")]
    TooManyEntries { max_entries: usize },

    #[error("snapshot subscription {index} has an invalid user URN")]
    InvalidUserUrn { index: usize },

    #[error("snapshot subscription {index} repeats a user URN")]
    DuplicateUserUrn { index: usize },

    #[error("snapshot exceeds its {max_bytes} byte limit")]
    TooLarge { max_bytes: usize },
}

#[cfg(test)]
mod tests;
