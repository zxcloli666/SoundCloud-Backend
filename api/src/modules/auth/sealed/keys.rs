use std::fs;

use anyhow::Context;
use hkdf::Hkdf;
use sha2::Sha256;

use crate::modules::auth::sealed::config::SealedCfg;
use crate::modules::chat::channel::ock::OckRing;

const SEED_KEY_INFO: &[u8] = b"scd-pow/seed-key/v1";

pub fn load(cfg: &SealedCfg) -> anyhow::Result<(OckRing, [u8; 32])> {
    let ring = OckRing::load(&cfg.keys_dir, cfg.ock_current)?;
    let path = cfg.keys_dir.join(format!("ock-{}", cfg.ock_current));
    let secret = fs::read(&path).with_context(|| format!("{} is not readable", path.display()))?;
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(None, &secret)
        .expand(SEED_KEY_INFO, &mut key)
        .map_err(|_| anyhow::anyhow!("the seed key does not fit"))?;
    Ok((ring, key))
}
