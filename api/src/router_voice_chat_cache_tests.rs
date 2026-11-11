use super::*;

const MARK: &str = "prefix_cache = { type = \"ephemeral\" }";

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn the_shared_sections_go_as_marked_blocks_that_repeat_from_turn_to_turn(
    pool: PgPool,
) -> anyhow::Result<()> {
    let vendor = Vendor::open().await?;
    let spoken = ["{\"actions\":[],\"say\":\"Привет.\"}"];
    vendor.model(
        "paid-model",
        Reply::Stream(vendor::message(&spoken, 0, 1_000, 100), End::Closes),
    );
    let llm = provider(
        "paid",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "paid-model",
        MARK,
    ) + &ladder(&["paid"], "")
        + CHAT_SECTION;
    let app = starred(&pool, &llm).await?;
    let first_user = session_for(&pool, &fresh_user()).await?;
    let second_user = session_for(&pool, &fresh_user()).await?;
    let mut later = turn();
    later["text"] = json!("а теперь тише");
    later["context"] = json!({"volume": 80, "now_playing": {"title": "Night Drive"}});
    later["notes"] = json!([]);
    let mut other_voice = turn();
    other_voice["persona"] = json!("default");

    for (session, body) in [
        (first_user, turn()),
        (second_user, later),
        (first_user, other_voice),
    ] {
        let heard = ask(&app, session, &body).await?;
        assert_eq!(heard.named().last().unwrap().0, "done");
    }

    let calls = vendor.calls();
    let sent: Vec<&Vec<Value>> = calls
        .iter()
        .map(|call| call.body["system"].as_array().expect("system blocks"))
        .collect();
    let [first, second, third] = sent.as_slice() else {
        panic!("three calls: {}", sent.len());
    };
    for blocks in [first, second, third] {
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0]["cache_control"], json!({"type": "ephemeral"}));
        assert_eq!(blocks[1]["cache_control"], json!({"type": "ephemeral"}));
        assert!(blocks[2].get("cache_control").is_none());
    }
    assert_eq!(first[0], second[0]);
    assert_eq!(first[1], second[1]);
    assert_ne!(first[2], second[2]);
    assert_eq!(first[0], third[0]);
    assert_ne!(first[1], third[1]);

    let text = |block: &Value| block["text"].as_str().unwrap_or_default().to_owned();
    assert!(text(&first[0]).starts_with("You are the voice of a music player app."));
    assert!(text(&first[0]).contains("- volume_set(level: integer 0..100)"));
    assert!(text(&first[0]).contains("Keep it short."));
    assert!(text(&first[1]).starts_with("\n\nYour character:\nA cheerful voice named Loli."));
    assert!(text(&third[1]).starts_with("\n\nYour character:\nA friendly voice."));
    assert!(text(&first[2]).starts_with("\n\nWhat the user asked you to remember:"));
    assert!(text(&second[2]).starts_with("\n\nApp state right now:"));
    Ok(())
}
