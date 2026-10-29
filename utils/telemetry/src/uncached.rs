use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Metadata};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, Filter};

pub(crate) struct Uncached<F>(pub(crate) F);

impl<S, F: Filter<S>> Filter<S> for Uncached<F> {
    fn enabled(&self, meta: &Metadata<'_>, cx: &Context<'_, S>) -> bool {
        self.0.enabled(meta, cx)
    }

    fn callsite_enabled(&self, meta: &'static Metadata<'static>) -> Interest {
        if self.0.callsite_enabled(meta).is_never() {
            Interest::never()
        } else {
            Interest::sometimes()
        }
    }

    fn event_enabled(&self, event: &Event<'_>, cx: &Context<'_, S>) -> bool {
        self.0.event_enabled(event, cx)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        self.0.max_level_hint()
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, cx: Context<'_, S>) {
        self.0.on_new_span(attrs, id, cx)
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, cx: Context<'_, S>) {
        self.0.on_record(id, values, cx)
    }

    fn on_enter(&self, id: &Id, cx: Context<'_, S>) {
        self.0.on_enter(id, cx)
    }

    fn on_exit(&self, id: &Id, cx: Context<'_, S>) {
        self.0.on_exit(id, cx)
    }

    fn on_close(&self, id: Id, cx: Context<'_, S>) {
        self.0.on_close(id, cx)
    }
}
