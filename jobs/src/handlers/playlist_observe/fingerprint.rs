use sha2::{Digest, Sha256};

pub fn membership_fingerprint(track_ids: &[String]) -> Vec<u8> {
    let mut digest = Sha256::new();
    digest.update((track_ids.len() as u64).to_be_bytes());
    for track_id in track_ids {
        digest.update((track_id.len() as u64).to_be_bytes());
        digest.update(track_id.as_bytes());
    }
    digest.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_preserves_order_and_boundaries() {
        let ordered = membership_fingerprint(&["1".to_owned(), "23".to_owned()]);
        let reordered = membership_fingerprint(&["23".to_owned(), "1".to_owned()]);
        let different_boundaries = membership_fingerprint(&["12".to_owned(), "3".to_owned()]);

        assert_ne!(ordered, reordered);
        assert_ne!(ordered, different_boundaries);
        assert_eq!(ordered.len(), 32);
    }
}
