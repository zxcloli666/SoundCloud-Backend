use std::cell::RefCell;
use std::fmt::Display;
use std::future::Future;

use tracing::field::Empty;
use tracing::{Instrument, Span};

tokio::task_local! {
    static FAILURE: RefCell<Option<(&'static str, String)>>;
}

pub(super) trait Attempt {
    fn served(&self) -> bool;

    fn error(&self) -> Option<String> {
        None
    }
}

impl<T> Attempt for Option<T> {
    fn served(&self) -> bool {
        self.is_some()
    }
}

impl<T, E: Display> Attempt for Result<Option<T>, E> {
    fn served(&self) -> bool {
        matches!(self, Ok(Some(_)))
    }

    fn error(&self) -> Option<String> {
        self.as_ref().err().map(ToString::to_string)
    }
}

pub(super) fn failed(outcome: &'static str, message: &str) {
    let _ = FAILURE.try_with(|failure| *failure.borrow_mut() = Some((outcome, message.to_owned())));
}

pub(super) async fn source<A: Attempt>(
    name: &'static str,
    tier: &'static str,
    attempt: impl Future<Output = A>,
) -> A {
    let span = tracing::info_span!(
        "source",
        otel.name = %format!("source {name}"),
        otel.status_code = Empty,
        otel.status_description = Empty,
        scd.source = name,
        scd.tier = tier,
        scd.outcome = Empty,
        error.message = Empty,
    );
    let (result, failure) = FAILURE
        .scope(RefCell::new(None), async {
            let result = attempt.await;
            (result, FAILURE.with(|failure| failure.take()))
        })
        .instrument(span.clone())
        .await;
    let failure = failure.or_else(|| result.error().map(|message| ("error", message)));
    if result.served() {
        span.record("scd.outcome", "served");
    } else if let Some((outcome, message)) = failure {
        record_failure(&span, outcome, &message);
    } else {
        span.record("scd.outcome", "missed");
    }
    result
}

fn record_failure(span: &Span, outcome: &str, message: &str) {
    span.record("scd.outcome", outcome);
    span.record("otel.status_code", "ERROR");
    span.record("otel.status_description", message);
    span.record("error.message", message);
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing::field::{Field, Visit};
    use tracing::span::Record;
    use tracing::{Id, Subscriber};
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{Layer, registry};

    use super::*;

    type Seen = Arc<Mutex<Vec<(String, String)>>>;

    struct Recorder(Seen);

    struct Fields<'a>(&'a Seen);

    impl Visit for Fields<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0
                .lock()
                .expect("lock")
                .push((field.name().to_owned(), format!("{value:?}")));
        }
    }

    impl<S: Subscriber> Layer<S> for Recorder {
        fn on_record(&self, _: &Id, values: &Record<'_>, _: Context<'_, S>) {
            values.record(&mut Fields(&self.0));
        }
    }

    fn run<T>(attempt: impl Future<Output = Option<T>>) -> Vec<(String, String)> {
        let seen: Seen = Arc::default();
        let subscriber = registry().with(Recorder(seen.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("runtime")
                .block_on(source("anon", "hq", attempt));
        });
        let recorded = seen.lock().expect("lock").clone();
        recorded
    }

    fn has(recorded: &[(String, String)], name: &str, value: &str) -> bool {
        recorded.iter().any(|(n, v)| n == name && v.contains(value))
    }

    #[test]
    fn a_source_that_answers_is_served() {
        let recorded = run(async { Some(1) });

        assert!(has(&recorded, "scd.outcome", "served"));
    }

    #[test]
    fn a_source_with_nothing_is_missed_and_not_an_error() {
        let recorded = run(async { None::<u8> });

        assert!(has(&recorded, "scd.outcome", "missed"));
        assert!(!recorded.iter().any(|(n, _)| n == "otel.status_code"));
    }

    #[test]
    fn a_source_that_timed_out_is_an_error_span() {
        let recorded = run(async {
            failed("timeout", "gave nothing in time");
            None::<u8>
        });

        assert!(has(&recorded, "scd.outcome", "timeout"));
        assert!(has(&recorded, "otel.status_code", "ERROR"));
        assert!(has(&recorded, "error.message", "gave nothing in time"));
    }
}
