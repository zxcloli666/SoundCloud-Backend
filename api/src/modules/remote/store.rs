use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::Context;
use async_nats::jetstream::kv::{self, CreateErrorKind, Operation};
use async_nats::jetstream::stream::{self, DirectGetErrorKind, DiscardPolicy, StorageType};
use backend_contracts::remote::{RemoteCommand, ReplyEnvelope};
use bytes::Bytes;
use uuid::Uuid;

use super::devices::Devices;
use crate::bus::nats::NatsService;
use crate::error::{AppError, AppResult};

const COMMANDS_AGE: Duration = Duration::from_secs(120);
const COMMANDS_PER_USER: i64 = 64;
const REPLIES_AGE: Duration = Duration::from_secs(600);
const DEVICES_AGE: Duration = Duration::from_secs(90);
const DEVICE_WRITE_TRIES: usize = 4;
const REQUEST_BUDGET: Duration = Duration::from_secs(2);

pub struct Store {
    nats: Arc<NatsService>,
    prefix: String,
    commands: stream::Stream,
    replies: kv::Store,
    devices: kv::Store,
}

pub struct Stored {
    pub seq: u64,
    pub command: Option<RemoteCommand>,
}

fn remote_error(what: &str, error: impl Display) -> AppError {
    AppError::internal(format!("remote {what}: {error}"))
}

async fn bucket(js: &Context, name: String, max_age: Duration) -> AppResult<kv::Store> {
    js.create_or_update_key_value(kv::Config {
        bucket: name,
        history: 1,
        max_age,
        storage: StorageType::File,
        ..Default::default()
    })
    .await
    .map_err(|error| remote_error("bucket", error))
}

impl Store {
    pub async fn open(nats: Arc<NatsService>, prefix: &str) -> AppResult<Self> {
        let mut js = nats.jetstream().clone();
        js.set_timeout(REQUEST_BUDGET);
        let commands = js
            .get_or_create_stream(stream::Config {
                name: format!("{prefix}_commands"),
                subjects: vec![format!("{prefix}.cmd.*")],
                max_age: COMMANDS_AGE,
                max_messages_per_subject: COMMANDS_PER_USER,
                discard: DiscardPolicy::Old,
                duplicate_window: COMMANDS_AGE,
                allow_direct: true,
                storage: StorageType::File,
                ..Default::default()
            })
            .await
            .map_err(|error| remote_error("commands stream", error))?;
        let replies = bucket(&js, format!("{prefix}_replies"), REPLIES_AGE).await?;
        let devices = bucket(&js, format!("{prefix}_devices"), DEVICES_AGE).await?;
        Ok(Self {
            nats,
            prefix: prefix.to_owned(),
            commands,
            replies,
            devices,
        })
    }

    fn command_subject(&self, user: &str) -> String {
        format!("{}.cmd.{user}", self.prefix)
    }

    fn reply_subject(&self, id: Uuid) -> String {
        format!("{}.reply.{id}", self.prefix)
    }

    fn reply_key(user: &str, id: Uuid) -> String {
        format!("{user}.{id}")
    }

    pub async fn publish(&self, command: &RemoteCommand) -> AppResult<()> {
        self.nats
            .publish_dedup(
                &self.command_subject(&command.user),
                command,
                &command.id.to_string(),
            )
            .await
    }

    pub async fn last_seq(&self) -> AppResult<u64> {
        let info = self
            .commands
            .get_info()
            .await
            .map_err(|error| remote_error("commands info", error))?;
        Ok(info.state.last_sequence)
    }

    pub async fn next_after(&self, user: &str, seq: u64) -> AppResult<Option<Stored>> {
        let read = self
            .commands
            .direct_get_next_for_subject(self.command_subject(user), Some(seq + 1))
            .await;
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
            Err(error) => Err(remote_error("command read", error)),
        }
    }

    pub async fn wake(&self, user: &str) -> AppResult<async_nats::Subscriber> {
        self.nats.subscribe(&self.command_subject(user)).await
    }

    pub async fn replies_to(&self, id: Uuid) -> AppResult<async_nats::Subscriber> {
        self.nats.subscribe(&self.reply_subject(id)).await
    }

    pub async fn record_reply(&self, envelope: &ReplyEnvelope) -> AppResult<()> {
        let body = Bytes::from(
            serde_json::to_vec(envelope).map_err(|error| remote_error("reply encode", error))?,
        );
        let key = Self::reply_key(&envelope.user, envelope.reply.id);
        match self.replies.create(key, body.clone()).await {
            Ok(_) => {}
            Err(error) if error.kind() == CreateErrorKind::AlreadyExists => return Ok(()),
            Err(error) => return Err(remote_error("reply store", error)),
        }
        self.nats
            .publish_core(&self.reply_subject(envelope.reply.id), body)
            .await
    }

    pub async fn reply_for(&self, user: &str, id: Uuid) -> AppResult<Option<ReplyEnvelope>> {
        let stored = self
            .replies
            .get(Self::reply_key(user, id))
            .await
            .map_err(|error| remote_error("reply read", error))?;
        Ok(stored
            .and_then(|body| serde_json::from_slice::<ReplyEnvelope>(&body).ok())
            .filter(|envelope| envelope.user == user && envelope.reply.id == id))
    }

    pub async fn devices(&self, user: &str) -> AppResult<Devices> {
        let stored = self
            .devices
            .get(user)
            .await
            .map_err(|error| remote_error("devices read", error))?;
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
    ) -> AppResult<()> {
        for _ in 0..DEVICE_WRITE_TRIES {
            let entry = self
                .devices
                .entry(user)
                .await
                .map_err(|error| remote_error("devices read", error))?;
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
                serde_json::to_vec(&devices)
                    .map_err(|error| remote_error("devices encode", error))?,
            );
            let written = match revision {
                Some(revision) => self.devices.update(user, body, revision).await.is_ok(),
                None => self.devices.create(user, body).await.is_ok(),
            };
            if written {
                return Ok(());
            }
        }
        Err(AppError::internal(
            "remote devices: the entry keeps changing",
        ))
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
