use backend_contracts::diag::Bundle;
use backend_contracts::diag_report::{
    Counts, Header, Missing, Report, ReportApp, ReportError, SourcemapState, View,
};
use serde_json::json;

use super::hints::{Facts, run};
use super::jaeger_view::{span, trace};
use super::{frames, public, timeline};

const TRACE: &str = "4bf92f3577b3da6a3ce929d0e0e4736f";
const SPAN: &str = "00f067aa0ba902b7";
const INSTALL: &str = "a1b2c3d4e5f6";
const LEAKY_HOP: &str = "r9 sc-call1 10.1.2.3";
const LEAKY_CREATED: &str = "10.9.8.7 sc-worker1 2026-10-29T10:00:30Z";

fn errors_of(bundle: &Bundle) -> Vec<ReportError> {
    bundle
        .errors
        .iter()
        .map(|error| ReportError {
            at: error.at.clone(),
            kind: error.kind,
            message: error.message.clone(),
            count: error.count,
            fingerprint: error.fingerprint.clone(),
            context: error.context.clone(),
            component_stack: error.component_stack.clone(),
            frames: error
                .stack
                .as_deref()
                .map(frames::parse)
                .unwrap_or_default(),
            issue: None,
            trace: error.trace.clone(),
        })
        .collect()
}

fn leaky_bundle(at: &str, hop: &str) -> Bundle {
    serde_json::from_value(json!({
        "kind": "scd-diagnostics", "v": 1, "at": at,
        "app": { "version": "8.4.25", "os": "windows", "os_version": "10", "arch": "x86_64", "install": "nsis", "locale": "ru", "id": INSTALL },
        "session": { "uptime_s": 1, "logged_in": true, "premium": false, "quality": "hq", "route": {} },
        "trigger": { "kind": "manual", "error": null },
        "attempts": [
            { "at": "2026-10-29T10:00:00.000Z", "trace": TRACE, "span": SPAN, "debug": true, "service": "stream",
              "method": "GET",
              "path": "/tracks/1/stream?client_id=AbCdEfGhIjKlMnOpQrStUvWxYz123456&oauth_token=2-290059-123456789-AbCdEfGhIjKl",
              "hop": hop, "status": 0, "ms": 30000,
              "error": format!("traceparent 00-{TRACE}-{SPAN}-01 session 0190f5a2-7c3e-7b8a-9d1e-2f3a4b5c6d7e jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl X-Amz-Signature=3f2a9c0d1e2b3a4f5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b peer addr:10.0.0.5 id_{INSTALL}"),
              "bytes": 0 },
            { "at": "2026-10-29T10:00:31.000Z", "trace": TRACE, "span": "00f067aa0ba902b8", "debug": true, "service": "stream",
              "method": "GET", "path": "/stream/soundcloud:tracks:1", "hop": "direct", "status": 200, "ms": 100,
              "error": null, "bytes": 1 }
        ],
        "errors": [{ "at": "2026-10-29T10:00:20.000Z", "kind": "render", "message": "boom", "stack": null,
                     "component_stack": null, "trace": null, "fingerprint": "0123456789abcdef", "count": 1, "context": {} }],
        "netcheck": null, "log_tail": []
    }))
    .expect("bundle")
}

#[test]
fn a_bundle_that_hides_internal_names_in_its_header_or_hop_is_refused() {
    assert!(leaky_bundle(LEAKY_CREATED, "direct").check().is_err());
    assert!(
        leaky_bundle("2026-10-29T10:00:30Z", LEAKY_HOP)
            .check()
            .is_err()
    );
    assert!(leaky_bundle("2026-10-29T10:00:30Z", "r9").check().is_ok());
}

#[test]
fn the_public_view_keeps_no_token_id_address_or_internal_name() {
    let bundle = leaky_bundle("2026-10-29T10:00:30Z", LEAKY_HOP);
    let mut root = span(
        "streaming-main",
        "main",
        "GET /stream/{id}",
        "1111111111111111",
        Some(SPAN),
        0,
        40_000,
        true,
        &[("scd.outcome", "exhausted")],
    );
    root.message = Some(
        "dial call-control:443 then vectors:6334, jobs:8080, pgbouncer:6432 via sc-call1"
            .to_owned(),
    );
    let slow = span(
        "streaming-main",
        "main",
        "source storage",
        "2222222222222222",
        Some("1111111111111111"),
        5,
        39_000,
        true,
        &[
            ("scd.source", "storage"),
            ("error.message", "s3 at s3.ovh.internal-net and sc-worker1"),
        ],
    );
    let traces = vec![trace(TRACE, vec![root, slow])];
    let timeline = timeline::build(&bundle, &traces);
    let errors = errors_of(&bundle);
    let hints = run(&Facts {
        bundle: &bundle,
        timeline: &timeline,
        traces: &traces,
        errors: &errors,
    });
    let mut report = Report {
        header: Header {
            view: View::Internal,
            code: "SCD-7F3K92QA".to_owned(),
            created: LEAKY_CREATED.to_owned(),
            app: ReportApp::from_app(&bundle.app),
            session: bundle.session.clone(),
        },
        counts: Counts::default(),
        offset_ms: timeline.offset_ms,
        timeline: timeline.entries,
        errors,
        hints,
        missing: Missing {
            traces: 0,
            trace_ids: vec![],
            sourcemaps: SourcemapState::NotNeeded,
        },
    };

    public::redact(&mut report);
    let text = serde_json::to_string(&report).expect("json");

    let leaked: Vec<&str> = [
        TRACE,
        SPAN,
        INSTALL,
        "AbCdEfGhIjKlMnOpQrStUvWxYz123456",
        "2-290059-123456789-AbCdEfGhIjKl",
        "0190f5a2-7c3e-7b8a-9d1e-2f3a4b5c6d7e",
        "eyJhbGciOiJIUzI1NiJ9",
        "3f2a9c0d1e2b3a4f5c6d7e8f9a0b1c2d",
        "10.0.0.5",
        "10.1.2.3",
        "10.9.8.7",
        "sc-call1",
        "sc-worker1",
        "call-control:443",
        "vectors:6334",
        "jobs:8080",
        "pgbouncer:6432",
        "source storage",
    ]
    .into_iter()
    .filter(|needle| text.contains(needle))
    .collect();
    assert!(leaked.is_empty(), "{leaked:?} in {text}");
    assert_eq!(report.header.created, "");
    assert!(text.contains("\"view\":\"public\""));
}

#[test]
fn the_public_header_is_rebuilt_from_the_parsed_time() {
    let bundle = leaky_bundle("2026-10-29T13:00:30+03:00", "direct");
    let mut report = Report {
        header: Header {
            view: View::Internal,
            code: "SCD-7F3K92QA".to_owned(),
            created: bundle.at.clone(),
            app: ReportApp::from_app(&bundle.app),
            session: bundle.session.clone(),
        },
        counts: Counts::default(),
        offset_ms: None,
        timeline: vec![],
        errors: vec![],
        hints: vec![],
        missing: Missing {
            traces: 0,
            trace_ids: vec![],
            sourcemaps: SourcemapState::NotNeeded,
        },
    };

    public::redact(&mut report);

    assert_eq!(report.header.created, "2026-10-29T10:00:30.000Z");
}
