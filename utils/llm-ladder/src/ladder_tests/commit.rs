use super::*;

const SAY: &str = "\"say\":\"";

fn speaks() -> Commit {
    Arc::new(|document: &str| {
        document.split_once(SAY).is_some_and(|(_, said)| {
            said.chars()
                .take_while(|symbol| *symbol != '"')
                .any(|symbol| !symbol.is_whitespace())
        })
    })
}

#[tokio::test(start_paused = true)]
async fn the_race_commits_on_the_first_visible_piece_and_hands_over_what_it_held() {
    let (first, _) = free(
        "first",
        vec![
            text(100, "{\"sa"),
            text(100, "y\":\""),
            text(100, "hi"),
            text(100, " there\"}"),
        ],
    );
    let bench = uncapped(vec![first]);

    let mut events = bench.ladder.stream(request(), speaks()).await.unwrap();
    let mut deltas = Vec::new();
    while let Some(Ok(Event::Delta(delta))) = events.next().await {
        deltas.push(delta);
    }

    assert_eq!(deltas, ["{\"say\":\"hi", " there\"}"]);
}

#[tokio::test(start_paused = true)]
async fn an_empty_answer_of_a_weak_step_falls_through_to_the_next_one() {
    let (weak, _) = free("weak", vec![text(100, "{\"say\":\"\"}")]);
    let (strong, _) = free("strong", vec![text(100, "{\"say\":\"here\"}")]);
    let bench = uncapped(vec![weak, strong]);

    let (document, outcome) = answer_once(&bench.ladder, speaks()).await;

    assert_eq!(document, "{\"say\":\"here\"}");
    assert_eq!(outcome.unwrap().elapsed, Duration::from_millis(200));
    assert_eq!(
        bench.seen.steps(),
        steps(&[("weak", "failed_invalid"), ("strong", "won")])
    );
}

#[tokio::test(start_paused = true)]
async fn an_empty_answer_is_charged_by_the_usage_that_follows_it() {
    let late_usage = (40, usage(1_000, 5).1);
    let (weak, _) = paid("weak", vec![text(100, "{\"say\":\"\"}"), late_usage]);
    let (strong, _) = free("strong", vec![text(100, "{\"say\":\"here\"}")]);
    let bench = bench(vec![weak, strong], monthly());

    let (document, outcome) = answer_once(&bench.ladder, speaks()).await;

    assert_eq!(document, "{\"say\":\"here\"}");
    assert_eq!(outcome.unwrap().elapsed, Duration::from_millis(240));
    let weak = &bench.seen.reports()[0];
    assert_eq!(
        (weak.result, weak.basis, weak.cost_micro_usd),
        ("failed_invalid", "reported", 2_040)
    );
}

#[tokio::test(start_paused = true)]
async fn an_empty_answer_that_keeps_the_line_gives_way_after_the_usage_wait() {
    let (weak, _) = hanging("weak", vec![text(100, "{\"say\":\"\"}")]);
    let (strong, _) = free("strong", vec![text(100, "{\"say\":\"here\"}")]);
    let bench = uncapped(vec![weak, strong]);

    let (document, outcome) = answer_once(&bench.ladder, speaks()).await;

    assert_eq!(document, "{\"say\":\"here\"}");
    assert_eq!(outcome.unwrap().elapsed, Duration::from_millis(1_200));
    assert_eq!(
        bench.seen.steps(),
        steps(&[("weak", "failed_invalid"), ("strong", "won")])
    );
}

#[tokio::test(start_paused = true)]
async fn a_line_that_breaks_after_an_empty_answer_is_not_held_against_the_provider() {
    let (weak, _) = free(
        "weak",
        vec![text(100, "{\"say\":\"\"}"), broken(10, Failure::Transport)],
    );
    let bench = uncapped(vec![weak]);

    let (_, outcome) = answer_once(&bench.ladder, speaks()).await;

    assert_eq!(outcome, Err(LadderError::Exhausted));
    assert_eq!(bench.seen.steps(), steps(&[("weak", "failed_invalid")]));
}

#[tokio::test(start_paused = true)]
async fn a_step_that_breaks_before_anything_visible_still_falls_through() {
    let (opener, _) = free(
        "opener",
        vec![text(100, "{\"say\":"), broken(100, Failure::Transport)],
    );
    let (closer, _) = free("closer", vec![text(100, "{\"say\":\"done\"}")]);
    let bench = uncapped(vec![opener, closer]);

    let (document, outcome) = answer_once(&bench.ladder, speaks()).await;

    assert_eq!(document, "{\"say\":\"done\"}");
    assert_eq!(outcome.unwrap().provider, "closer");
}

#[tokio::test(start_paused = true)]
async fn a_step_that_is_still_writing_its_answer_is_not_hedged() {
    let (writer, _) = free(
        "writer",
        vec![text(100, "{\"say\":"), text(4_000, "\"slowly\"}")],
    );
    let (spare, spare_probe) = free("spare", vec![text(1, "{\"say\":\"spare\"}")]);
    let bench = uncapped(vec![writer, spare]);

    let (document, _) = answer_once(&bench.ladder, speaks()).await;

    assert_eq!(document, "{\"say\":\"slowly\"}");
    assert_eq!(spare_probe.started(), 0);
}

#[tokio::test(start_paused = true)]
async fn when_no_step_says_anything_the_ladder_is_exhausted() {
    let (weak, _) = free("weak", vec![text(100, "{\"say\":\" \"}")]);
    let bench = uncapped(vec![weak]);

    let (document, outcome) = answer_once(&bench.ladder, speaks()).await;

    assert_eq!(document, "");
    assert_eq!(outcome, Err(LadderError::Exhausted));
}
