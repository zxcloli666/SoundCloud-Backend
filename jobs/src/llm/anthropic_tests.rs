use super::*;

fn step() -> Step {
    Step {
        id: "opus".to_owned(),
        endpoint: "https://llm.example/v1/messages".to_owned(),
        model: "example-model".to_owned(),
        key: Secret::new("k".to_owned()),
        headers: Vec::new(),
        extra: serde_json::from_value(json!({"output_config": {"effort": "high"}})).unwrap(),
        cache: serde_json::from_value(json!({"type": "ephemeral", "ttl": "1h"})).unwrap(),
        max_tokens: 16_000,
        timeout: Duration::from_secs(300),
        prices: Prices {
            input: 5.0,
            output: 25.0,
            cache_read: 0.5,
            cache_write: 10.0,
        },
    }
}

fn tools() -> Value {
    json!([{"name": "a"}, {"name": "b"}, {"name": "c"}])
}

#[test]
fn the_system_prompt_and_the_last_tool_carry_the_cache_mark_and_config_extras_merge_in() {
    let call = Call {
        name: "review",
        system: Some("system"),
        messages: &[],
        tools: Some(tools()),
        format: Some(json!({"type": "object"})),
        effort: None,
    };

    let body = step().body(&call);

    assert_eq!(body["system"][0]["cache_control"]["ttl"], "1h");
    assert_eq!(body["tools"][2]["cache_control"]["type"], "ephemeral");
    assert!(body["tools"][0].get("cache_control").is_none());
    assert_eq!(body["tool_choice"]["type"], "auto");
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(body["output_config"]["effort"], "high");
    assert_eq!(body["output_config"]["format"]["type"], "json_schema");
}

#[test]
fn a_call_without_a_system_prompt_sends_none_and_its_effort_goes_into_the_output_config() {
    let mut plain = step();
    plain.extra = Map::new();
    let call = Call {
        name: "vlm",
        system: None,
        messages: &[],
        tools: None,
        format: None,
        effort: Some("low"),
    };

    let body = plain.body(&call);

    assert!(body.get("system").is_none());
    assert!(body.get("tools").is_none());
    assert_eq!(body["output_config"], json!({"effort": "low"}));
    assert_eq!(body["thinking"], json!({"type": "adaptive"}));
}

#[test]
fn spend_is_priced_from_real_usage_and_the_config_prices() {
    let usage = Usage {
        input_tokens: 2_000,
        output_tokens: 1_000,
        cache_read_input_tokens: 10_000,
        cache_creation_input_tokens: 1_000,
    };

    let spend = step().spend("plan", &usage);

    assert!((spend.usd - (0.01 + 0.025 + 0.005 + 0.01)).abs() < 1e-9);
    assert_eq!(
        (spend.provider.as_str(), spend.cache_read_tokens),
        ("opus", 10_000)
    );
}

#[test]
fn limits_and_server_errors_are_down_and_other_statuses_are_invalid() {
    for status in [401, 403, 429, 500, 529] {
        assert!(matches!(answer_of(status, b"{}"), Err(LlmError::Down(_))));
    }
    assert!(matches!(answer_of(400, b"bad"), Err(LlmError::Invalid(_))));
    assert!(matches!(
        answer_of(200, b"not json"),
        Err(LlmError::Invalid(_))
    ));
}

#[test]
fn a_refusal_or_a_cut_answer_is_not_finished() {
    let message = |stop: &str| Message {
        content: Vec::new(),
        stop_reason: Some(stop.to_owned()),
        usage: Usage::default(),
    };

    assert!(finished(&message("refusal")).is_err());
    assert!(finished(&message("max_tokens")).is_err());
    assert!(finished(&message("end_turn")).is_ok());
}
