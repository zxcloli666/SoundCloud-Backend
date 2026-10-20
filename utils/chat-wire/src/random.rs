use hpke::rand_core::{CryptoRng, RngCore};

use crate::WireError;

pub fn random_bytes<const N: usize>() -> Result<[u8; N], WireError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|_| WireError::Random)?;
    Ok(bytes)
}

pub(crate) struct Seeded {
    seed: [u8; 32],
    at: usize,
}

impl Seeded {
    pub(crate) fn new(seed: [u8; 32]) -> Self {
        Self { seed, at: 0 }
    }
}

impl RngCore for Seeded {
    fn next_u32(&mut self) -> u32 {
        let mut bytes = [0u8; 4];
        self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }

    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0u8; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    fn fill_bytes(&mut self, dst: &mut [u8]) {
        for byte in dst {
            *byte = self.seed[self.at % self.seed.len()];
            self.at += 1;
        }
    }
}

impl CryptoRng for Seeded {}
