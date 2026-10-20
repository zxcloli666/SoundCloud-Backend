use uuid::Uuid;

use super::*;

#[test]
fn every_key_keeps_the_layout_the_plan_fixed() {
    let conv = Uuid::from_u128(0x0192_e5c5_1a2b_7c3d_8e4f_5a6b_7c8d_9e0f);
    let session = Uuid::from_u128(1);
    assert_eq!(version("101"), "chat:v:101");
    assert_eq!(deltas("101"), "chat:d:101");
    assert_eq!(
        conversation_seq(conv),
        "chat:c:0192e5c5-1a2b-7c3d-8e4f-5a6b7c8d9e0f"
    );
    assert_eq!(
        typing(conv, "101"),
        "chat:t:0192e5c5-1a2b-7c3d-8e4f-5a6b7c8d9e0f:101"
    );
    assert_eq!(
        read_marks(conv),
        "chat:r:0192e5c5-1a2b-7c3d-8e4f-5a6b7c8d9e0f"
    );
    assert_eq!(
        delivered_marks(conv),
        "chat:dl:0192e5c5-1a2b-7c3d-8e4f-5a6b7c8d9e0f"
    );
    assert_eq!(
        slow_mode(conv, "lt:7"),
        "chat:slow:0192e5c5-1a2b-7c3d-8e4f-5a6b7c8d9e0f:lt:7"
    );
    assert_eq!(limit("send", "101"), "chat:lim:send:101");
    assert_eq!(
        loadtest_session(session),
        "chat:lt:sess:00000000-0000-0000-0000-000000000001"
    );
    assert_eq!(
        (ROOMS_ALIVE, SEEN, LEAD),
        ("rooms:alive", "chat:seen", "chat:lead")
    );
}

#[test]
fn the_replay_key_is_the_first_half_of_the_enc_digest() {
    let key = replay(&[0u8; 32]);
    assert_eq!(key, "chat:rp:66687aadf862bd776c8fc18b8e9f8e20");
    assert_ne!(replay(&[1u8; 32]), key);
}

#[test]
fn every_ttl_is_set() {
    for ttl in [
        ROOMS_TTL,
        VERSION_TTL,
        TYPING_TTL,
        MARKS_TTL,
        SEEN_TTL,
        REPLAY_TTL,
        LEAD_TTL,
        LOADTEST_SESSION_TTL,
    ] {
        assert!(!ttl.is_zero());
    }
    assert_eq!(REPLAY_TTL.as_secs(), 300);
    assert_eq!(TYPING_TTL.as_secs(), 6);
}
