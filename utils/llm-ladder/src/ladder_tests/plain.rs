use super::*;
use crate::request::Plain;

#[tokio::test(start_paused = true)]
async fn a_plain_answer_waits_while_a_step_with_tools_still_runs() {
    let (tools, _) = free("tools", vec![text(3_000, "{\"say\":\"tools\"}")]);
    let (local, local_probe) = plain(free("local", vec![text(1_000, "{\"say\":\"local\"}")]));
    let bench = grouped(vec![tools, local], vec![vec![0, 1]]);

    let (document, outcome) = answer_to(&bench.ladder, plainly_too(), any_text()).await;
    let outcome = outcome.unwrap();

    assert_eq!(document, "{\"say\":\"tools\"}");
    assert_eq!(outcome.provider, "tools");
    assert_eq!(outcome.elapsed, Duration::from_millis(3_000));
    assert_eq!(local_probe.dropped(), 1);
    assert_eq!(
        bench.seen.steps(),
        steps(&[("local", "lost"), ("tools", "won")])
    );
}

#[tokio::test(start_paused = true)]
async fn a_plain_answer_wins_once_no_step_with_tools_is_left() {
    let (tools, _) = free("tools", vec![broken(2_500, Failure::Status)]);
    let (local, _) = plain(free("local", vec![text(1_000, "{\"say\":\"local\"}")]));
    let bench = grouped(vec![tools, local], vec![vec![0, 1]]);

    let (document, outcome) = answer_to(&bench.ladder, plainly_too(), any_text()).await;
    let outcome = outcome.unwrap();

    assert_eq!(document, "{\"say\":\"local\"}");
    assert_eq!(outcome.provider, "local");
    assert_eq!(outcome.elapsed, Duration::from_millis(2_500));
    assert_eq!(
        bench.seen.steps(),
        steps(&[("tools", "failed_status"), ("local", "won")])
    );
}

#[tokio::test(start_paused = true)]
async fn a_plain_answer_alone_in_the_race_wins_at_once() {
    let (local, _) = plain(free("local", vec![text(1_000, "{\"say\":\"local\"}")]));
    let (tools, tools_probe) = free("tools", vec![text(100, "{\"say\":\"tools\"}")]);
    let bench = grouped(vec![local, tools], vec![vec![0], vec![1]]);

    let (document, outcome) = answer_to(&bench.ladder, plainly_too(), any_text()).await;

    assert_eq!(document, "{\"say\":\"local\"}");
    assert_eq!(outcome.unwrap().elapsed, Duration::from_millis(1_000));
    assert_eq!(tools_probe.started(), 0);
    assert_eq!(bench.seen.steps(), steps(&[("local", "won")]));
}

#[tokio::test(start_paused = true)]
async fn a_waiting_plain_answer_is_taken_at_the_deadline() {
    let (silent, _) = hanging("silent", vec![]);
    let (talking, _) = hanging("talking", vec![text(100, "{\"say\":")]);
    let (local, _) = plain(free("local", vec![text(3_000, "{\"say\":\"local\"}")]));
    let bench = grouped(vec![silent, talking, local], vec![vec![0], vec![1, 2]]);
    let whole: Commit = Arc::new(|_, held: &str| held.ends_with('}'));

    let (document, outcome) = answer_to(&bench.ladder, plainly_too(), whole).await;

    assert_eq!(document, "{\"say\":\"local\"}");
    assert_eq!(outcome.unwrap().elapsed, DEADLINE);
    assert_eq!(
        bench.seen.steps(),
        steps(&[
            ("silent", "failed_timeout"),
            ("talking", "lost"),
            ("local", "won"),
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn a_waiting_plain_answer_that_breaks_off_fails_its_step() {
    let (tools, _) = free("tools", vec![text(3_000, "{\"say\":\"tools\"}")]);
    let (local, _) = plain(free("local", vec![text(1_000, "{\"say\":\"lo")]));
    let bench = grouped(vec![tools, local], vec![vec![0, 1]]);

    let (document, outcome) = answer_to(&bench.ladder, plainly_too(), any_text()).await;

    assert_eq!(document, "{\"say\":\"tools\"}");
    assert_eq!(outcome.unwrap().provider, "tools");
    assert_eq!(
        bench.seen.steps(),
        steps(&[("local", "failed_invalid"), ("tools", "won")])
    );
}

fn plain((mut provider, probe): (Provider, Probe)) -> (Provider, Probe) {
    provider.plain = true;
    (provider, probe)
}

fn plainly_too() -> Request {
    Request {
        plain: Some(Plain {
            system: System::fixed("plain rules"),
            messages: vec![Message::user("hello")],
            schema: Schema::object([("say", Schema::String)]),
        }),
        ..request()
    }
}
