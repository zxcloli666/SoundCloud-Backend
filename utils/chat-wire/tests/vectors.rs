use chat_wire::WireError;
use chat_wire::bind::{bind_string, make_bind, verify_bind};
use chat_wire::ck;
use chat_wire::msg::{self, Class, MessageAad};
use chat_wire::ock::OckSecret;
use chat_wire::prost::Message;
use chat_wire::proto::{Bind, Envelope, Header, Inner, InnerResponse};
use chat_wire::seal::{RequestSealer, ResponseKey, open_request};
use chat_wire::sign::{public_key, request_string, sign_request, verify_request};
use serde_json::Value;

fn vector(name: &str) -> Value {
    let path = format!("{}/vectors/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("vector file")).expect("vector json")
}

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().expect("hex string")).expect("valid hex")
}

fn array<const N: usize>(value: &Value) -> [u8; N] {
    bytes(value).try_into().expect("right length")
}

fn uuid(value: &Value) -> [u8; 16] {
    let text = value.as_str().expect("uuid string").replace('-', "");
    hex::decode(text)
        .expect("uuid hex")
        .try_into()
        .expect("16 bytes")
}

fn text(value: &Value) -> String {
    value.as_str().expect("string").to_owned()
}

fn inner_of(case: &Value) -> Inner {
    let inner = &case["inner"];
    Inner {
        ts_ms: inner["ts_ms"].as_u64().expect("ts_ms"),
        session_id: inner["session_id"].as_str().map(str::to_owned),
        device_id: inner["device_id"].as_str().map(str::to_owned),
        method: text(&inner["method"]),
        path: text(&inner["path"]),
        query: text(&inner["query"]),
        body: text(&inner["body"]).into_bytes(),
        tier: inner["tier"].as_i64().expect("tier") as i32,
        max_bytes: inner["max_bytes"].as_u64().expect("max_bytes") as u32,
        bind: inner["bind"]
            .as_str()
            .map(|raw| Bind::decode(hex::decode(raw).expect("bind hex").as_slice()).expect("bind")),
        sig: Vec::new(),
    }
}

fn response_of(case: &Value) -> InnerResponse {
    let response = &case["response"];
    InnerResponse {
        status: response["status"].as_u64().expect("status") as u32,
        headers: response["headers"]
            .as_array()
            .expect("headers")
            .iter()
            .map(|pair| Header {
                name: text(&pair[0]),
                value: text(&pair[1]),
            })
            .collect(),
        body: text(&response["body"]).into_bytes(),
    }
}

#[test]
fn every_request_vector_seals_signs_and_opens_to_the_recorded_bytes() {
    let cases = vector("request.json");
    for case in cases["cases"].as_array().expect("cases") {
        let name = text(&case["name"]);
        let ock = OckSecret::from_bytes(&bytes(&case["ock_secret"])).expect("ock secret");
        assert_eq!(ock.public(), array::<32>(&case["ock_pub"]), "{name}");
        let signer = array::<32>(&case["signer_seed"]);
        assert_eq!(
            public_key(&signer),
            array::<32>(&case["signer_pub"]),
            "{name}"
        );

        let sealer = RequestSealer::new(
            case["ock_v"].as_u64().expect("ock_v") as u8,
            &ock.public(),
            array(&case["ikm_e"]),
        )
        .expect("sealer");
        assert_eq!(sealer.enc(), &array::<32>(&case["enc"]), "{name}");

        let mut inner = inner_of(case);
        sign_request(&signer, &mut inner, sealer.enc());
        assert_eq!(
            request_string(&inner, sealer.enc()),
            text(&case["sign_string"]),
            "{name}"
        );
        assert_eq!(inner.sig, bytes(&case["sig"]), "{name}");
        assert_eq!(inner.encode_to_vec(), bytes(&case["inner_bytes"]), "{name}");
        verify_request(&public_key(&signer), &inner, sealer.enc()).expect("signature verifies");

        let sealed = sealer.seal(&inner.encode_to_vec()).expect("seal");
        assert_eq!(sealed.envelope, bytes(&case["envelope"]), "{name}");
        assert_eq!(
            sealed.response_key.as_bytes(),
            &array::<32>(&case["response_key"]),
            "{name}"
        );

        let opened = open_request(&sealed.envelope, &ock).expect("open");
        assert_eq!(opened.inner, bytes(&case["inner_bytes"]), "{name}");
        assert_eq!(opened.enc, array::<32>(&case["enc"]), "{name}");
        assert_eq!(
            opened.response_key.as_bytes(),
            sealed.response_key.as_bytes(),
            "{name}"
        );

        let answer = response_of(case);
        assert_eq!(
            answer.encode_to_vec(),
            bytes(&case["response_bytes"]),
            "{name}"
        );
        let response = opened
            .response_key
            .seal(
                &opened.enc,
                array(&case["response_nonce"]),
                &answer.encode_to_vec(),
            )
            .expect("seal response");
        assert_eq!(response, bytes(&case["response_sealed"]), "{name}");
        let key = ResponseKey::from_bytes(array(&case["response_key"]));
        let plain = key.open(&opened.enc, &response).expect("open response");
        assert_eq!(
            InnerResponse::decode(plain.as_slice()).expect("decode"),
            answer,
            "{name}"
        );
    }
}

#[test]
fn a_login_vector_is_signed_by_the_key_inside_its_bind() {
    let cases = vector("request.json");
    let login = cases["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|case| case["name"] == "login")
        .expect("a login case");
    let inner = Inner {
        sig: bytes(&login["sig"]),
        ..inner_of(login)
    };
    let bind = inner.bind.as_ref().expect("a bind");
    verify_bind(bind).expect("bind verifies");
    verify_request(&bind.auth_pub, &inner, &array(&login["enc"])).expect("signed by bind key");
    assert!(inner.session_id.is_none() && inner.device_id.is_none());
    assert!(
        text(&login["sign_string"])
            .ends_with(&format!("\n-\n-\n{}", sha256_hex(&bytes(&login["enc"]))))
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    chat_wire::sign::sha256_hex(bytes)
}

#[test]
fn a_tampered_envelope_or_answer_never_opens() {
    let cases = vector("request.json");
    let case = &cases["cases"][0];
    let ock = OckSecret::from_bytes(&bytes(&case["ock_secret"])).expect("ock");
    let envelope = bytes(&case["envelope"]);

    for at in [1, 2, 40, envelope.len() - 1] {
        let mut tampered = envelope.clone();
        tampered[at] ^= 1;
        assert!(
            open_request(&tampered, &ock).is_err(),
            "byte {at} was not covered"
        );
    }
    let mut wrong_version = envelope.clone();
    wrong_version[0] = 2;
    assert_eq!(
        open_request(&wrong_version, &ock).err(),
        Some(WireError::Version(2))
    );
    assert_eq!(
        open_request(&envelope[..30], &ock).err(),
        Some(WireError::Envelope)
    );
    let other = OckSecret::from_bytes(&[7u8; 32]).expect("other key");
    assert_eq!(open_request(&envelope, &other).err(), Some(WireError::Open));

    let key = ResponseKey::from_bytes(array(&case["response_key"]));
    let enc = array::<32>(&case["enc"]);
    let mut answer = bytes(&case["response_sealed"]);
    let last = answer.len() - 1;
    answer[last] ^= 1;
    assert!(key.open(&enc, &answer).is_err());
    let mut other_enc = enc;
    other_enc[0] ^= 1;
    assert!(
        key.open(&other_enc, &bytes(&case["response_sealed"]))
            .is_err()
    );
}

#[test]
fn a_signature_does_not_move_to_another_request() {
    let cases = vector("request.json");
    let case = &cases["cases"][0];
    let signer_pub = array::<32>(&case["signer_pub"]);
    let enc = array::<32>(&case["enc"]);
    let signed = Inner {
        sig: bytes(&case["sig"]),
        ..inner_of(case)
    };
    verify_request(&signer_pub, &signed, &enc).expect("the original verifies");

    let moved_path = Inner {
        path: "/chat/devices".into(),
        ..signed.clone()
    };
    assert!(verify_request(&signer_pub, &moved_path, &enc).is_err());
    let moved_body = Inner {
        body: b"{}".to_vec(),
        ..signed.clone()
    };
    assert!(verify_request(&signer_pub, &moved_body, &enc).is_err());
    let mut other_enc = enc;
    other_enc[5] ^= 1;
    assert!(verify_request(&signer_pub, &signed, &other_enc).is_err());
    let smuggled = Inner {
        query: "a=1\nb".into(),
        ..signed
    };
    assert!(verify_request(&signer_pub, &smuggled, &enc).is_err());
}

#[test]
fn the_bind_vector_rebuilds_byte_for_byte() {
    let vector = vector("bind.json");
    let auth = array::<32>(&vector["auth_seed"]);
    let box_secret = array::<32>(&vector["box_secret"]);
    let box_pub = ck::box_public(&box_secret).expect("box pub");
    assert_eq!(box_pub, array::<32>(&vector["box_pub"]));
    let device_id = text(&vector["device_id"]);
    let ts_ms = vector["ts_ms"].as_u64().expect("ts");
    let bind = make_bind(&device_id, &auth, &box_pub, ts_ms);
    assert_eq!(bind.auth_pub, bytes(&vector["auth_pub"]));
    assert_eq!(
        bind_string(&device_id, &bind.auth_pub, &bind.box_pub, ts_ms),
        text(&vector["sign_string"])
    );
    assert_eq!(bind.sig, bytes(&vector["sig"]));
    assert_eq!(bind.encode_to_vec(), bytes(&vector["bind_bytes"]));
    verify_bind(&bind).expect("verifies");

    let swapped = Bind {
        box_pub: vec![9; 32],
        ..bind
    };
    assert_eq!(verify_bind(&swapped).err(), Some(WireError::Signature));
}

#[test]
fn the_ck_vector_wraps_and_seals_byte_for_byte() {
    let vector = vector("ck.json");
    let kek = array::<32>(&vector["kek"]);
    let kek_v = vector["kek_v"].as_u64().expect("kek_v") as u16;
    let conv = uuid(&vector["conversation_id"]);
    let epoch = vector["epoch"].as_u64().expect("epoch") as u32;
    let key = array::<32>(&vector["ck"]);
    let device = uuid(&vector["device_id"]);

    assert_eq!(
        ck::wrap_aad(&conv, epoch, kek_v),
        bytes(&vector["wrap_aad"])
    );
    let wrapped = ck::wrap(
        &kek,
        kek_v,
        &conv,
        epoch,
        &key,
        array(&vector["wrap_nonce"]),
    )
    .expect("wrap");
    assert_eq!(wrapped, bytes(&vector["wrapped"]));
    assert_eq!(
        ck::unwrap(&kek, kek_v, &conv, epoch, &wrapped).expect("unwrap"),
        key
    );
    assert!(ck::unwrap(&kek, kek_v, &conv, epoch + 1, &wrapped).is_err());
    assert!(ck::unwrap(&kek, kek_v + 1, &conv, epoch, &wrapped).is_err());

    let box_secret = array::<32>(&vector["box_secret"]);
    assert_eq!(
        ck::seal_info(&conv, epoch, &device),
        bytes(&vector["seal_info"])
    );
    let sealed = ck::seal_for_device(
        &bytes(&vector["box_pub"]),
        &conv,
        epoch,
        &device,
        &key,
        array(&vector["ikm_e"]),
    )
    .expect("seal");
    assert_eq!(sealed, bytes(&vector["sealed"]));
    assert_eq!(
        ck::open_for_device(&box_secret, &conv, epoch, &device, &sealed).expect("open"),
        key
    );
    let mut other_device = device;
    other_device[0] ^= 1;
    assert!(ck::open_for_device(&box_secret, &conv, epoch, &other_device, &sealed).is_err());
}

#[test]
fn every_message_vector_pads_and_seals_byte_for_byte() {
    let vector = vector("msg.json");
    for case in vector["cases"].as_array().expect("cases") {
        let name = text(&case["name"]);
        let key = array::<32>(&case["ck"]);
        let class = Class::parse(case["class"].as_str().expect("class")).expect("known class");
        let aad = MessageAad {
            conv: uuid(&case["conversation_id"]),
            epoch: case["epoch"].as_u64().expect("epoch") as u32,
            sender_device: uuid(&case["sender_device"]),
            client_id: uuid(&case["client_id"]),
            class,
        };
        assert_eq!(aad.bytes(), bytes(&case["aad"]), "{name}");

        let plaintext = bytes(&case["plaintext"]);
        assert_eq!(plaintext.len() % msg::PAD_BLOCK, 0, "{name}");
        let envelope = Envelope::decode(plaintext.as_slice()).expect("plaintext decodes");
        assert_eq!(msg::encode_padded(&envelope), plaintext, "{name}");

        let body = msg::seal(&key, &aad, array(&case["nonce"]), &envelope).expect("seal");
        assert_eq!(body, bytes(&case["body"]), "{name}");
        assert_eq!(
            msg::open(&key, &aad, &body).expect("open"),
            envelope,
            "{name}"
        );

        let wrong_class = MessageAad {
            class: if class == Class::Post {
                Class::Edit
            } else {
                Class::Post
            },
            ..aad
        };
        assert!(msg::open(&key, &wrong_class, &body).is_err(), "{name}");
    }
}

#[test]
fn the_text_vector_carries_the_described_envelope() {
    let vector = vector("msg.json");
    let case = &vector["cases"][0];
    let envelope = Envelope::decode(bytes(&case["plaintext"]).as_slice()).expect("decodes");
    let description = &case["envelope"];
    let Some(chat_wire::proto::envelope::Body::Text(body)) = &envelope.body else {
        panic!("the first case is a text");
    };
    assert_eq!(body.text, text(&description["text"]["text"]));
    assert_eq!(body.spans.len(), 2);
    assert_eq!(body.spans[0].start, 8);
    assert_eq!(body.spans[1].len, 3);
    assert_eq!(envelope.reply_to, 118);
    assert_eq!(
        envelope.embeds[0].urn,
        text(&description["embeds"][0]["urn"])
    );
}

#[test]
fn padding_lands_on_a_block_for_every_length() {
    for size in 0..1200 {
        let envelope = Envelope {
            v: 1,
            body: Some(chat_wire::proto::envelope::Body::Text(
                chat_wire::proto::Text {
                    text: "x".repeat(size),
                    spans: Vec::new(),
                },
            )),
            ..Default::default()
        };
        let padded = msg::encode_padded(&envelope);
        assert_eq!(padded.len() % msg::PAD_BLOCK, 0, "size {size}");
        let decoded = Envelope::decode(padded.as_slice()).expect("decodes");
        assert_eq!(decoded.body, envelope.body, "size {size}");
    }
}

#[test]
fn an_unknown_body_decodes_to_none() {
    let mut future = Envelope {
        v: 1,
        ..Default::default()
    }
    .encode_to_vec();
    future.extend_from_slice(&[0x4a, 0x02, 0x08, 0x01]);
    let decoded = Envelope::decode(future.as_slice()).expect("decodes");
    assert!(decoded.body.is_none());
}

#[test]
fn the_production_pins_are_the_published_keys() {
    let pins = chat_wire::ock::OCK_PINS;
    assert_eq!(pins.len(), 2);
    assert_eq!(
        hex::encode(chat_wire::ock::pinned(1).expect("pin 1")),
        "46b0bccd368d8553a5e09bc8aff285142c58be72454504d01dd7cd964991ee63"
    );
    assert_eq!(
        hex::encode(chat_wire::ock::pinned(2).expect("pin 2")),
        "a4ba96381c8a03bf977799af43ae573eba75d787a47acb7cc2b6622595d43054"
    );
    assert!(chat_wire::ock::pinned(3).is_none());
}

#[test]
fn a_fresh_request_opens_with_fresh_randomness() {
    let ock = OckSecret::from_bytes(&[3u8; 32]).expect("ock");
    let first = RequestSealer::new(1, &ock.public(), chat_wire::random_bytes().expect("rng"))
        .expect("sealer");
    let second = RequestSealer::new(1, &ock.public(), chat_wire::random_bytes().expect("rng"))
        .expect("sealer");
    assert_ne!(first.enc(), second.enc());
    let sealed = first.seal(b"inner").expect("seal");
    assert_eq!(
        open_request(&sealed.envelope, &ock).expect("open").inner,
        b"inner"
    );
}
