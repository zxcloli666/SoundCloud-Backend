use sc_chat_wire::ock::OckSecret;
use sc_chat_wire::prost::Message;
use sc_chat_wire::proto::{Inner, InnerResponse, Tier};
use sc_chat_wire::seal::open_request;
use sc_chat_wire::sign::{public_key, verify_request};
use uuid::Uuid;

use super::{Call, Ock, seal};
use crate::layout::Identity;

fn identity() -> Identity {
    Identity {
        index: 3,
        user: "lt:4".to_owned(),
        device: Uuid::now_v7(),
        session: Uuid::new_v4(),
        auth_seed: [9u8; 32],
        box_secret: [8u8; 32],
    }
}

#[test]
fn a_sealed_call_opens_with_the_ock_and_carries_a_valid_signature() {
    let secret = OckSecret::from_bytes(&[5u8; 32]).expect("an ock");
    let ock = Ock::from_hex(1, &hex::encode(secret.public())).expect("a hex key");
    let identity = identity();
    let call = Call {
        method: "POST",
        path: "/chat/sync",
        query: "",
        body: br#"{"hold_ms":0}"#.to_vec(),
    };
    let sealed = seal(
        &identity,
        &ock,
        call,
        Tier::Pro,
        24 * 1024,
        1_700_000_000_000,
    )
    .expect("sealed");

    let opened = open_request(&sealed.envelope, &secret).expect("the server opens it");
    let inner = Inner::decode(opened.inner.as_slice()).expect("an inner");
    assert_eq!(inner.path, "/chat/sync");
    assert_eq!(inner.tier, Tier::Pro as i32);
    assert_eq!(inner.max_bytes, 24 * 1024);
    assert_eq!(inner.session_id, Some(identity.session.to_string()));
    assert_eq!(inner.device_id, Some(identity.device.to_string()));
    verify_request(&public_key(&identity.auth_seed), &inner, &opened.enc)
        .expect("signed by the device");

    let answer = InnerResponse {
        status: 204,
        headers: Vec::new(),
        body: Vec::new(),
    };
    let body = opened
        .response_key
        .seal(&opened.enc, [1u8; 24], &answer.encode_to_vec())
        .expect("a sealed answer");
    assert_eq!(sealed.open(&body).expect("the answer opens").status, 204);
    assert!(sealed.open(b"forged").is_err());
}

#[test]
fn the_pinned_ock_is_the_default_and_a_wrong_key_is_refused() {
    assert_eq!(
        Ock::pinned(1).expect("pinned").public,
        *sc_chat_wire::ock::pinned(1).expect("a pin")
    );
    assert!(Ock::pinned(9).is_err());
    assert!(Ock::from_hex(1, "abcd").is_err());
    assert!(Ock::from_hex(1, "zz").is_err());
}
