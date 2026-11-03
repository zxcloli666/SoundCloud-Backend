use opentelemetry::Context;
use opentelemetry::trace::{SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState};

pub struct Incoming {
    pub trace: TraceId,
    pub span: SpanId,
    pub flags: u8,
}

impl Incoming {
    pub fn debug(&self) -> bool {
        self.flags & 1 == 1
    }

    pub fn context(&self) -> Context {
        let remote = SpanContext::new(
            self.trace,
            self.span,
            TraceFlags::new(self.flags),
            true,
            TraceState::default(),
        );
        Context::new().with_remote_span_context(remote)
    }
}

pub fn parse(value: &str) -> Option<Incoming> {
    let mut parts = value.trim().split('-');
    let version = parts.next()?;
    let trace = parts.next()?;
    let span = parts.next()?;
    let flags = parts.next()?;
    if version.len() != 2 || version == "ff" || !is_hex(version) {
        return None;
    }
    if version == "00" && parts.next().is_some() {
        return None;
    }
    if trace.len() != 32 || span.len() != 16 || flags.len() != 2 {
        return None;
    }
    if ![trace, span, flags].iter().all(|part| is_hex(part)) {
        return None;
    }
    let trace = TraceId::from_hex(trace).ok()?;
    let span = SpanId::from_hex(span).ok()?;
    if trace == TraceId::INVALID || span == SpanId::INVALID {
        return None;
    }
    Some(Incoming {
        trace,
        span,
        flags: u8::from_str_radix(flags, 16).ok()?,
    })
}

fn is_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "00-4bf92f3577b3da6a3ce929d0e0e4736f-00f067aa0ba902b7-01";

    #[test]
    fn a_valid_header_is_read() {
        let incoming = parse(GOOD).expect("valid");

        assert_eq!(
            incoming.trace.to_string(),
            "4bf92f3577b3da6a3ce929d0e0e4736f"
        );
        assert_eq!(incoming.span.to_string(), "00f067aa0ba902b7");
        assert!(incoming.debug());
    }

    #[test]
    fn flags_zero_is_not_debug() {
        let incoming = parse(&GOOD.replace("-01", "-00")).expect("valid");

        assert!(!incoming.debug());
    }

    #[test]
    fn garbage_starts_a_new_trace() {
        for bad in [
            "",
            "garbage",
            "00-4bf92f3577b3da6a3ce929d0e0e4736f-00f067aa0ba902b7",
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b3da6a3ce929d0e0e4736f-0000000000000000-01",
            "00-4BF92F3577B3DA6A3CE929D0E0E4736F-00f067aa0ba902b7-01",
            "ff-4bf92f3577b3da6a3ce929d0e0e4736f-00f067aa0ba902b7-01",
            "00-4bf92f3577b3da6a3ce929d0e0e4736f-00f067aa0ba902b7-01-extra",
        ] {
            assert!(parse(bad).is_none(), "{bad}");
        }
    }
}
