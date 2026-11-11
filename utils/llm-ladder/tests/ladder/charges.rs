use crate::bench::{Bench, answer, bench, http_provider, ladder};
use crate::vendor::{COMPLETIONS, MESSAGES, Reply, Step, Vendor};
use llm_ladder::LadderError;
use serde_json::json;

const PRICE: &str = "input_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0";

async fn failed(bench: &Bench, ladder: &str) -> Step {
    let ladder = bench.ladders.get(ladder).expect("the ladder exists");
    assert_eq!(answer(&ladder).await.outcome, Err(LadderError::Exhausted));
    bench.seen.steps().pop().expect("the step was settled")
}

#[tokio::test]
async fn only_a_failure_before_the_provider_took_the_request_is_free() {
    let vendor = Vendor::open().await;
    let one_shot = format!("stream = false\n{PRICE}");
    let config = http_provider(
        "whole",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "whole-model",
        &one_shot,
    ) + &http_provider(
        "stream",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "stream-model",
        PRICE,
    ) + &ladder("once", &["whole"], 500)
        + &ladder("live", &["stream"], 500);
    let bench = bench(&config, None);

    vendor.model(
        "whole-model",
        Reply::Status(
            529,
            json!({"type": "error", "error": {"type": "overloaded_error"}}),
        ),
    );
    let refused = failed(&bench, "once").await;
    assert_eq!(
        (refused.result, refused.basis, refused.cost_micro_usd),
        ("failed_status", "reported", 0)
    );

    vendor.model("whole-model", Reply::Whole(0, json!("not a message")));
    vendor.model("stream-model", Reply::Breaks(Vec::new()));
    for ladder in ["once", "live"] {
        let broke = failed(&bench, ladder).await;
        assert_eq!(
            (broke.result, broke.basis),
            ("failed_transport", "estimated"),
            "{ladder}"
        );
        assert!(broke.usage.input_tokens > 0, "{broke:?}");
        assert_eq!(broke.cost_micro_usd, broke.usage.input_tokens * 2);
    }
}
