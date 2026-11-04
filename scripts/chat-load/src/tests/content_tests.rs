use rand::SeedableRng;
use rand::rngs::StdRng;
use uuid::Uuid;

use super::{Sealing, Stamp, open, read_stamp, seal, text};

#[test]
fn a_stamp_survives_the_text_and_the_cipher() {
    let stamp = Stamp {
        run: 77,
        sender: 1_234,
        sent_ms: 98_765,
    };
    let written = text(stamp, &mut StdRng::seed_from_u64(1));
    assert_eq!(read_stamp(&written), Some(stamp));

    let ck = [6u8; 32];
    let sealing = Sealing {
        ck: &ck,
        conversation: Uuid::now_v7(),
        epoch: 0,
        sender_device: Uuid::now_v7(),
        client_id: Uuid::now_v7(),
    };
    let body = seal(&sealing, written.clone()).expect("sealed");
    assert_eq!((body.len() - 24 - 16) % 256, 0);
    assert_eq!(open(&sealing, &body).expect("opened"), written);

    let other = Sealing {
        client_id: Uuid::now_v7(),
        ..sealing
    };
    assert!(open(&other, &body).is_err());
}

#[test]
fn foreign_text_has_no_stamp() {
    assert_eq!(read_stamp("hello"), None);
    assert_eq!(read_stamp("lt|x|1|2|y"), None);
    assert_eq!(read_stamp("lt|1|2"), None);
}
