use std::error::Error;
use std::fmt;
use std::time::Duration;

use async_nats::HeaderMap;
use async_nats::jetstream::Context;
use async_nats::jetstream::kv::{self, CreateErrorKind, Operation};
use async_nats::jetstream::stream::{self, DirectGetErrorKind, DiscardPolicy, StorageType};
use backend_contracts::remote::{RemoteCommand, ReplyEnvelope};
use bytes::Bytes;
use uuid::Uuid;

use super::devices::Devices;
use crate::bus::nats::NatsService;

pub const COMMANDS_AGE: Duration = Duration::from_secs(120);
const COMMANDS_PER_USER: i64 = 64;
pub const SCAN_LIMIT: usize = COMMANDS_PER_USER as usize;
pub const COMMANDS_BYTES: i64 = 64 * 1024 * 1024;
pub const MAX_COMMAND_BYTES: i32 = 4 * 1024;
const REPLIES_AGE: Duration = Duration::from_secs(600);
pub const REPLIES_BYTES: i64 = 64 * 1024 * 1024;
pub const MAX_REPLY_VALUE: i32 = 16 * 1024;
const DEVICES_AGE: Duration = Duration::from_secs(90);
const DEVICES_BYTES: i64 = 16 * 1024 * 1024;
const MAX_DEVICES_VALUE: i32 = 4 * 1024;
const CLAIMS_BYTES: i64 = 16 * 1024 * 1024;
const MAX_CLAIM_VALUE: i32 = 512;
const DEVICE_WRITE_TRIES: usize = 4;
const REQUEST_BUDGET: Duration = Duration::from_secs(2);
const MISSING_MARKERS: [&str; 4] = [
    "stream not found",
    "no stream found",
    "does not exist",
    "no responders",
];

pub struct Store {
    js: Context,
    prefix: String,
    commands: stream::Stream,
    replies: kv::Store,
    devices: kv::Store,
    claims: kv::Store,
}

pub struct Stored {
    pub seq: u64,
    pub command: Option<RemoteCommand>,
}

#[derive(Debug)]
pub struct StoreError {
    pub missing: bool,
    message: String,
}

pub type StoreResult<T> = Result<T, StoreError>;

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

fn failed(what: &str, error: impl Error) -> StoreError {
    let mut chain = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        chain.push_str(": ");
        chain.push_str(&inner.to_string());
        source = inner.source();
    }
    let lowered = chain.to_lowercase();
    StoreError {
        missing: MISSING_MARKERS
            .iter()
            .any(|marker| lowered.contains(marker)),
        message: format!("remote {what}: {chain}"),
    }
}

async fn in_budget<F: Future>(what: &str, request: F) -> StoreResult<F::Output> {
    tokio::time::timeout(REQUEST_BUDGET, request)
        .await
        .map_err(|_| StoreError {
            missing: false,
            message: format!(
                "remote {what}: no answer in {} ms",
                REQUEST_BUDGET.as_millis()
            ),
        })
}

pub fn commands_stream(prefix: &str) -> String {
    format!("{prefix}_commands")
}

pub fn bucket_name(prefix: &str, kind: &str) -> String {
    format!("{prefix}_{kind}")
}

struct Bucket {
    name: String,
    max_age: Duration,
    max_bytes: i64,
    max_value_size: i32,
}

async fn bucket(js: &Context, spec: Bucket) -> StoreResult<kv::Store> {
    let created = js.create_or_update_key_value(kv::Config {
        bucket: spec.name,
        history: 1,
        max_age: spec.max_age,
        max_bytes: spec.max_bytes,
        max_value_size: spec.max_value_size,
        storage: StorageType::File,
        ..Default::default()
    });
    in_budget("bucket", created)
        .await?
        .map_err(|error| failed("bucket", error))
}

impl Store {
    pub async fn open(nats: &NatsService, prefix: &str) -> StoreResult<Self> {
        let mut js = nats.jetstream().clone();
        js.set_timeout(REQUEST_BUDGET);
        let created = js.create_or_update_stream(stream::Config {
            name: commands_stream(prefix),
            subjects: vec![format!("{prefix}.cmd.*")],
            max_age: COMMANDS_AGE,
            max_messages_per_subject: COMMANDS_PER_USER,
            max_bytes: COMMANDS_BYTES,
            max_message_size: MAX_COMMAND_BYTES,
            discard: DiscardPolicy::Old,
            duplicate_window: COMMANDS_AGE,
            allow_direct: true,
            storage: StorageType::File,
            ..Default::default()
        });
        in_budget("commands stream", created)
            .await?
            .map_err(|error| failed("commands stream", error))?;
        let commands = in_budget("commands stream", js.get_stream(commands_stream(prefix)))
            .await?
            .map_err(|error| failed("commands stream", error))?;
        let replies = bucket(
            &js,
            Bucket {
                name: bucket_name(prefix, "replies"),
                max_age: REPLIES_AGE,
                max_bytes: REPLIES_BYTES,
                max_value_size: MAX_REPLY_VALUE,
            },
        )
        .await?;
        let devices = bucket(
            &js,
            Bucket {
                name: bucket_name(prefix, "devices"),
                max_age: DEVICES_AGE,
                max_bytes: DEVICES_BYTES,
                max_value_size: MAX_DEVICES_VALUE,
            },
        )
        .await?;
        let claims = bucket(
            &js,
            Bucket {
                name: bucket_name(prefix, "claims"),
                max_age: COMMANDS_AGE,
                max_bytes: CLAIMS_BYTES,
                max_value_size: MAX_CLAIM_VALUE,
            },
        )
        .await?;
        Ok(Self {
            js,
            prefix: prefix.to_owned(),
            commands,
            replies,
            devices,
            claims,
        })
    }

    fn command_subject(&self, user: &str) -> String {
        format!("{}.cmd.{user}", self.prefix)
    }

    fn reply_subject(&self, id: Uuid) -> String {
        format!("{}.reply.{id}", self.prefix)
    }

    fn command_key(user: &str, id: Uuid) -> String {
        format!("{user}.{id}")
    }

    pub async fn publish(&self, command: &RemoteCommand) -> StoreResult<()> {
        let body = Bytes::from(
            serde_json::to_vec(command).map_err(|error| failed("command encode", error))?,
        );
        let mut headers = HeaderMap::new();
        headers.insert("Nats-Msg-Id", command.id.to_string().as_str());
        let acknowledgement = self
            .js
            .publish_with_headers(self.command_subject(&command.user), headers, body)
            .await
            .map_err(|error| failed("command publish", error))?;
        acknowledgement
            .await
            .map_err(|error| failed("command ack", error))?;
        Ok(())
    }

    pub async fn last_seq(&self) -> StoreResult<u64> {
        let info = in_budget("commands info", self.commands.get_info())
            .await?
            .map_err(|error| failed("commands info", error))?;
        Ok(info.state.last_sequence)
    }

    pub async fn next_after(&self, user: &str, seq: u64) -> StoreResult<Option<Stored>> {
        let read = in_budget(
            "command read",
            self.commands
                .direct_get_next_for_subject(self.command_subject(user), Some(seq + 1)),
        )
        .await?;
        match read {
            Ok(message) => {
                let command = serde_json::from_slice::<RemoteCommand>(&message.payload)
                    .ok()
                    .map(|mut command| {
                        command.seq = message.sequence as i64;
                        command
                    });
                Ok(Some(Stored {
                    seq: message.sequence,
                    command,
                }))
            }
            Err(error) if error.kind() == DirectGetErrorKind::NotFound => Ok(None),
            Err(error) => Err(failed("command read", error)),
        }
    }

    pub async fn wake(&self, user: &str) -> StoreResult<async_nats::Subscriber> {
        self.js
            .client()
            .subscribe(self.command_subject(user))
            .await
            .map_err(|error| failed("wake subscription", error))
    }

    pub async fn replies_to(&self, id: Uuid) -> StoreResult<async_nats::Subscriber> {
        self.js
            .client()
            .subscribe(self.reply_subject(id))
            .await
            .map_err(|error| failed("reply subscription", error))
    }

    pub async fn record_reply(&self, envelope: &ReplyEnvelope) -> StoreResult<()> {
        let body = Bytes::from(
            serde_json::to_vec(envelope).map_err(|error| failed("reply encode", error))?,
        );
        let key = Self::command_key(&envelope.user, envelope.reply.id);
        let first = match in_budget("reply store", self.replies.create(&key, body.clone())).await? {
            Ok(_) => body,
            Err(error) if error.kind() == CreateErrorKind::AlreadyExists => {
                match in_budget("reply read", self.replies.get(&key))
                    .await?
                    .map_err(|error| failed("reply read", error))?
                {
                    Some(stored) => stored,
                    None => return Ok(()),
                }
            }
            Err(error) => return Err(failed("reply store", error)),
        };
        self.js
            .client()
            .publish(self.reply_subject(envelope.reply.id), first)
            .await
            .map_err(|error| failed("reply publish", error))
    }

    pub async fn reply_for(&self, user: &str, id: Uuid) -> StoreResult<Option<ReplyEnvelope>> {
        let stored = in_budget("reply read", self.replies.get(Self::command_key(user, id)))
            .await?
            .map_err(|error| failed("reply read", error))?;
        Ok(stored
            .and_then(|body| serde_json::from_slice::<ReplyEnvelope>(&body).ok())
            .filter(|envelope| envelope.user == user && envelope.reply.id == id))
    }

    pub async fn claim(&self, user: &str, id: Uuid, device: &str) -> StoreResult<bool> {
        let key = Self::command_key(user, id);
        let mine = Bytes::copy_from_slice(device.as_bytes());
        match in_budget("claim", self.claims.create(&key, mine.clone())).await? {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == CreateErrorKind::AlreadyExists => {
                let holder = in_budget("claim read", self.claims.get(&key))
                    .await?
                    .map_err(|error| failed("claim read", error))?;
                Ok(holder.is_some_and(|holder| holder == mine))
            }
            Err(error) => Err(failed("claim", error)),
        }
    }

    pub async fn claimed(&self, user: &str, id: Uuid) -> StoreResult<bool> {
        let holder = in_budget("claim read", self.claims.get(Self::command_key(user, id)))
            .await?
            .map_err(|error| failed("claim read", error))?;
        Ok(holder.is_some())
    }

    pub async fn devices(&self, user: &str) -> StoreResult<Devices> {
        let stored = in_budget("devices read", self.devices.get(user))
            .await?
            .map_err(|error| failed("devices read", error))?;
        Ok(stored
            .and_then(|body| serde_json::from_slice(&body).ok())
            .unwrap_or_default())
    }

    pub async fn note_device(
        &self,
        user: &str,
        device: &str,
        active: i64,
        now_ms: i64,
    ) -> StoreResult<()> {
        let mut last = None;
        for _ in 0..DEVICE_WRITE_TRIES {
            let entry = in_budget("devices read", self.devices.entry(user))
                .await?
                .map_err(|error| failed("devices read", error))?;
            let (mut devices, revision) = match entry {
                Some(entry) if entry.operation == Operation::Put => (
                    serde_json::from_slice(&entry.value).unwrap_or_default(),
                    Some(entry.revision),
                ),
                Some(entry) => (Devices::default(), Some(entry.revision)),
                None => (Devices::default(), None),
            };
            devices.note(device, active, now_ms);
            let body = Bytes::from(
                serde_json::to_vec(&devices).map_err(|error| failed("devices encode", error))?,
            );
            let written = match revision {
                Some(revision) => {
                    in_budget("devices write", self.devices.update(user, body, revision))
                        .await?
                        .map(|_| ())
                        .map_err(|error| failed("devices write", error))
                }
                None => in_budget("devices write", self.devices.create(user, body))
                    .await?
                    .map(|_| ())
                    .map_err(|error| failed("devices write", error)),
            };
            match written {
                Ok(()) => return Ok(()),
                Err(error) if error.missing => return Err(error),
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| StoreError {
            missing: false,
            message: "remote devices: the entry keeps changing".to_owned(),
        }))
    }

    #[cfg(test)]
    pub async fn device_revision(&self, user: &str) -> Option<u64> {
        self.devices
            .entry(user)
            .await
            .ok()
            .flatten()
            .map(|entry| entry.revision)
    }
}
