use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const EPOCH_SECONDS: u64 = 300;

const SEED_DOMAIN: &[u8] = b"scd-pow/seed/v1";

pub struct Seeds {
    key: [u8; 32],
    node: String,
}

pub fn epoch_now() -> u64 {
    u64::try_from(Utc::now().timestamp()).unwrap_or(0) / EPOCH_SECONDS
}

impl Seeds {
    pub fn new(key: [u8; 32], node: String) -> Self {
        Self { key, node }
    }

    pub fn seed(&self, epoch: u64) -> [u8; 32] {
        self.mac(epoch).finalize().into_bytes().into()
    }

    pub fn accepts(&self, seed: &[u8], epoch: u64) -> bool {
        let previous = epoch.saturating_sub(1);
        [epoch, previous]
            .into_iter()
            .any(|epoch| self.mac(epoch).verify_slice(seed).is_ok())
    }

    fn mac(&self, epoch: u64) -> Hmac<Sha256> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("hmac takes any key length");
        mac.update(SEED_DOMAIN);
        mac.update(self.node.as_bytes());
        mac.update(&epoch.to_be_bytes());
        mac
    }
}
