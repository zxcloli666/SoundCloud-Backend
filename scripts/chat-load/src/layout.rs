use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, bail};
use serde::Deserialize;
use uuid::Uuid;

use crate::b64;

#[derive(Debug, Clone, Deserialize)]
pub struct Seeded {
    pub devices: Vec<SeededDevice>,
    pub conversations: Vec<SeededConversation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeededDevice {
    pub user: String,
    pub id: Uuid,
    pub session: Uuid,
    pub auth_seed: String,
    pub box_secret: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeededConversation {
    pub id: Uuid,
    pub members: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Identity {
    pub index: usize,
    pub user: String,
    pub device: Uuid,
    pub session: Uuid,
    pub auth_seed: [u8; 32],
    pub box_secret: [u8; 32],
}

#[derive(Debug, Default)]
pub struct Layout {
    pub identities: Vec<Identity>,
    pub chats: HashMap<String, Vec<Uuid>>,
}

impl Layout {
    pub fn load(paths: &[PathBuf]) -> anyhow::Result<Self> {
        let mut layout = Layout::default();
        for path in paths {
            let raw = std::fs::read(path)
                .with_context(|| format!("layout {} is not readable", path.display()))?;
            let seeded: Seeded = serde_json::from_slice(&raw)
                .with_context(|| format!("layout {} is not a seed answer", path.display()))?;
            layout.add(seeded)?;
        }
        if layout.identities.is_empty() {
            bail!("the layout holds no devices");
        }
        Ok(layout)
    }

    pub fn add(&mut self, seeded: Seeded) -> anyhow::Result<()> {
        for device in seeded.devices {
            let index = self.identities.len();
            self.identities.push(Identity {
                index,
                auth_seed: key(&device.auth_seed).context("auth_seed")?,
                box_secret: key(&device.box_secret).context("box_secret")?,
                user: device.user,
                device: device.id,
                session: device.session,
            });
        }
        for conversation in seeded.conversations {
            for member in conversation.members {
                self.chats.entry(member).or_default().push(conversation.id);
            }
        }
        Ok(())
    }

    pub fn chats_of(&self, user: &str) -> Vec<Uuid> {
        self.chats.get(user).cloned().unwrap_or_default()
    }
}

fn key(encoded: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = b64::decode(encoded).context("not base64")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("not a 32 byte key"))
}
