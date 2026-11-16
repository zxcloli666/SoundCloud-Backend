use super::*;

fn naming_say() -> Request {
    Request {
        text_field: Some("say"),
        ..request()
    }
}

#[tokio::test(start_paused = true)]
async fn plain_text_becomes_the_named_field_when_the_request_names_one() {
    let (think, _) = free(
        "think",
        vec![
            text(100, "<think>not for me</think>Это не ко мне."),
            text(50, " Слушаем дальше?"),
        ],
    );
    let (fast, fast_probe) = free("fast", vec![text(5_000, "{\"say\":\"fast\"}")]);
    let bench = uncapped(vec![think, fast]);

    let (document, outcome) = answer_to(&bench.ladder, naming_say(), any_text()).await;
    let outcome = outcome.unwrap();

    assert_eq!(document, "{\"say\":\"Это не ко мне. Слушаем дальше?\"}");
    assert_eq!(outcome.provider, "think");
    assert_eq!(outcome.elapsed, Duration::from_millis(150));
    assert_eq!(fast_probe.started(), 0);
    assert_eq!(bench.seen.steps(), steps(&[("think", "won")]));
}

#[tokio::test(start_paused = true)]
async fn plain_text_fails_the_step_when_the_request_names_no_field() {
    let (think, _) = free("think", vec![text(100, "Это не ко мне.")]);
    let (fast, _) = free("fast", vec![text(500, "{\"say\":\"fast\"}")]);
    let bench = uncapped(vec![think, fast]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"fast\"}");
    assert_eq!(outcome.unwrap().elapsed, Duration::from_millis(600));
    assert_eq!(
        bench.seen.steps(),
        steps(&[("think", "failed_invalid"), ("fast", "won")])
    );
}

#[tokio::test(start_paused = true)]
async fn thinking_or_blanks_alone_are_never_taken_for_text() {
    for answer in ["<think>maybe I should say hi</think>", "  \n "] {
        let (think, _) = free("think", vec![text(100, answer)]);
        let (fast, _) = free("fast", vec![text(500, "{\"say\":\"fast\"}")]);
        let bench = uncapped(vec![think, fast]);

        let (document, outcome) = answer_to(&bench.ladder, naming_say(), any_text()).await;

        assert_eq!(document, "{\"say\":\"fast\"}", "{answer}");
        assert_eq!(outcome.unwrap().provider, "fast");
        assert_eq!(
            bench.seen.steps()[0],
            ("think".to_owned(), "failed_invalid")
        );
    }
}

#[tokio::test(start_paused = true)]
async fn plain_text_still_has_to_pass_the_commit() {
    let (think, _) = free("think", vec![text(100, "Это не ко мне.")]);
    let (fast, _) = free("fast", vec![text(500, "{\"say\":\"fast\"}")]);
    let bench = uncapped(vec![think, fast]);
    let only_fast: Commit = Arc::new(|_, held: &str| held.contains("fast"));

    let (document, outcome) = answer_to(&bench.ladder, naming_say(), only_fast).await;

    assert_eq!(document, "{\"say\":\"fast\"}");
    assert_eq!(outcome.unwrap().provider, "fast");
    assert_eq!(
        bench.seen.steps(),
        steps(&[("think", "failed_invalid"), ("fast", "won")])
    );
}
