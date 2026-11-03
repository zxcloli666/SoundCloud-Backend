use backend_contracts::diag::Bundle;
use backend_contracts::diag_report::{
    Counts, Header, IssueStats, Missing, Report, ReportApp, ReportError, Rule, SourcemapState, View,
};
use serde_json::{Value, json};

use super::hints::{Facts, run};
use super::jaeger_view::{span, trace};
use super::{public, timeline};

const T1: &str = "4bf92f3577b3da6a3ce929d0e0e4736f";
const T2: &str = "5bf92f3577b3da6a3ce929d0e0e4736f";

fn attempt(
    trace: &str,
    span: &str,
    hop: &str,
    status: u16,
    error: Option<&str>,
    at: &str,
) -> Value {
    json!({
        "at": at, "trace": trace, "span": span, "debug": true, "service": "stream",
        "method": "GET", "path": "/stream/soundcloud:tracks:123?quality=hq", "hop": hop,
        "status": status, "ms": 120, "error": error, "bytes": 0
    })
}

fn bundle(attempts: Vec<Value>, errors: Vec<Value>) -> Bundle {
    serde_json::from_value(json!({
        "kind": "scd-diagnostics", "v": 1, "at": "2026-10-29T10:00:30.000Z",
        "app": { "version": "8.4.25", "os": "windows", "os_version": "10", "arch": "x86_64", "install": "nsis", "locale": "ru", "id": "a1b2c3d4e5f6" },
        "session": { "uptime_s": 1, "logged_in": true, "premium": false, "quality": "hq", "route": { "main": "relay:r2" } },
        "trigger": { "kind": "manual", "error": null },
        "attempts": attempts, "errors": errors, "netcheck": null, "log_tail": []
    }))
    .expect("a bundle")
}

fn error(fingerprint: &str) -> Value {
    json!({
        "at": "2026-10-29T10:00:20.000Z", "kind": "render", "message": "boom", "stack": null,
        "component_stack": null, "trace": null, "fingerprint": fingerprint, "count": 1, "context": {}
    })
}

fn stats(installs: i64, installs_24h: i64, first_version: &str) -> IssueStats {
    IssueStats {
        events: 5,
        installs,
        installs_24h,
        first_at: "2026-10-28T00:00:00Z".to_owned(),
        last_at: "2026-10-29T00:00:00Z".to_owned(),
        first_version: first_version.to_owned(),
        versions: Default::default(),
    }
}

fn error_row(stats: Option<IssueStats>) -> ReportError {
    ReportError {
        at: "2026-10-29T10:00:20.000Z".to_owned(),
        kind: backend_contracts::diag::ErrorKind::Render,
        message: "boom".to_owned(),
        count: 1,
        fingerprint: "0123456789abcdef".to_owned(),
        context: Default::default(),
        component_stack: None,
        frames: Vec::new(),
        issue: stats,
        trace: None,
    }
}

fn rules(
    bundle: &Bundle,
    traces: &[super::super::jaeger::TraceView],
    errors: &[ReportError],
) -> Vec<Rule> {
    let timeline = timeline::build(bundle, traces);
    run(&Facts {
        bundle,
        timeline: &timeline,
        traces,
        errors,
    })
    .into_iter()
    .map(|hint| hint.rule)
    .collect()
}

fn fires(
    rule: Rule,
    bundle: &Bundle,
    traces: &[super::super::jaeger::TraceView],
    errors: &[ReportError],
) {
    assert!(
        rules(bundle, traces, errors).contains(&rule),
        "{rule:?} should fire"
    );
}

fn quiet(
    rule: Rule,
    bundle: &Bundle,
    traces: &[super::super::jaeger::TraceView],
    errors: &[ReportError],
) {
    assert!(
        !rules(bundle, traces, errors).contains(&rule),
        "{rule:?} should stay quiet"
    );
}

const AT: &str = "2026-10-29T10:00:00.000Z";

#[test]
fn relay_blocked_fires_when_one_relay_fails_at_connect_and_direct_works() {
    let failing = bundle(
        vec![
            attempt(
                T1,
                "aaaaaaaaaaaaaaa1",
                "r2",
                0,
                Some("connect timeout after 10s"),
                AT,
            ),
            attempt(T1, "aaaaaaaaaaaaaaa2", "direct", 200, None, AT),
        ],
        vec![],
    );
    fires(Rule::RelayBlocked, &failing, &[], &[]);
    quiet(Rule::OriginBlocked, &failing, &[], &[]);

    let fine = bundle(
        vec![attempt(T1, "aaaaaaaaaaaaaaa1", "r2", 200, None, AT)],
        vec![],
    );
    quiet(Rule::RelayBlocked, &fine, &[], &[]);
    let both_down = bundle(
        vec![
            attempt(T1, "aaaaaaaaaaaaaaa1", "r2", 0, Some("connect"), AT),
            attempt(T1, "aaaaaaaaaaaaaaa2", "direct", 0, Some("connect"), AT),
        ],
        vec![],
    );
    quiet(Rule::RelayBlocked, &both_down, &[], &[]);
}

#[test]
fn origin_blocked_fires_when_direct_fails_and_a_relay_works() {
    let failing = bundle(
        vec![
            attempt(T1, "aaaaaaaaaaaaaaa1", "direct", 0, Some("connect"), AT),
            attempt(T1, "aaaaaaaaaaaaaaa2", "r1", 200, None, AT),
        ],
        vec![],
    );
    fires(Rule::OriginBlocked, &failing, &[], &[]);
    quiet(Rule::RelayBlocked, &failing, &[], &[]);
    let direct_fine = bundle(
        vec![attempt(T1, "aaaaaaaaaaaaaaa1", "direct", 200, None, AT)],
        vec![],
    );
    quiet(Rule::OriginBlocked, &direct_fine, &[], &[]);
}

#[test]
fn all_routes_down_needs_two_hops_and_no_answer() {
    let down = bundle(
        vec![
            attempt(T1, "aaaaaaaaaaaaaaa1", "direct", 0, Some("dns"), AT),
            attempt(T1, "aaaaaaaaaaaaaaa2", "r2", 0, Some("dns"), AT),
        ],
        vec![],
    );
    fires(Rule::AllRoutesDown, &down, &[], &[]);
    let one_hop = bundle(
        vec![attempt(
            T1,
            "aaaaaaaaaaaaaaa1",
            "direct",
            0,
            Some("dns"),
            AT,
        )],
        vec![],
    );
    quiet(Rule::AllRoutesDown, &one_hop, &[], &[]);
    let one_works = bundle(
        vec![
            attempt(T1, "aaaaaaaaaaaaaaa1", "direct", 0, Some("dns"), AT),
            attempt(T1, "aaaaaaaaaaaaaaa2", "r2", 200, None, AT),
        ],
        vec![],
    );
    quiet(Rule::AllRoutesDown, &one_works, &[], &[]);
}

#[test]
fn never_reached_fires_without_a_matching_server_span() {
    let failed = bundle(
        vec![attempt(
            T1,
            "aaaaaaaaaaaaaaa1",
            "r2",
            502,
            Some("bad gateway"),
            AT,
        )],
        vec![],
    );
    fires(Rule::NeverReached, &failed, &[], &[]);
    let seen = trace(
        T1,
        vec![span(
            "api-main",
            "main",
            "GET /x",
            "1111111111111111",
            Some("aaaaaaaaaaaaaaa1"),
            0,
            50,
            true,
            &[],
        )],
    );
    quiet(Rule::NeverReached, &failed, &[seen], &[]);
    let fine = bundle(
        vec![attempt(T1, "aaaaaaaaaaaaaaa1", "r2", 200, None, AT)],
        vec![],
    );
    quiet(Rule::NeverReached, &fine, &[], &[]);
}

#[test]
fn our_server_error_names_the_message_of_a_failed_trace() {
    let failed = bundle(
        vec![attempt(
            T1,
            "aaaaaaaaaaaaaaa1",
            "direct",
            500,
            Some("HTTP 500"),
            AT,
        )],
        vec![],
    );
    let mut root = span(
        "api-main",
        "main",
        "GET /x",
        "1111111111111111",
        Some("aaaaaaaaaaaaaaa1"),
        0,
        50,
        true,
        &[],
    );
    root.message = Some("pool timed out".to_owned());
    let traces = vec![trace(T1, vec![root])];
    fires(Rule::OurServerError, &failed, &traces, &[]);

    let healthy = bundle(
        vec![attempt(T1, "aaaaaaaaaaaaaaa1", "direct", 200, None, AT)],
        vec![],
    );
    quiet(Rule::OurServerError, &healthy, &traces, &[]);
    let ok_span = trace(
        T1,
        vec![span(
            "api-main",
            "main",
            "GET /x",
            "1111111111111111",
            Some("aaaaaaaaaaaaaaa1"),
            0,
            50,
            false,
            &[],
        )],
    );
    quiet(Rule::OurServerError, &failed, &[ok_span], &[]);
}

#[test]
fn our_server_error_names_the_request_with_its_status() {
    let failed = bundle(
        vec![attempt(T1, "aaaaaaaaaaaaaaa1", "direct", 503, None, AT)],
        vec![],
    );
    let root = span(
        "api-main",
        "main",
        "GET /resolve",
        "1111111111111111",
        Some("aaaaaaaaaaaaaaa1"),
        0,
        50,
        true,
        &[("http.response.status_code", "503")],
    );
    let traces = vec![trace(T1, vec![root])];
    let hints = run(&Facts {
        bundle: &failed,
        timeline: &timeline::build(&failed, &traces),
        traces: &traces,
        errors: &[],
    });

    let hint = hints
        .iter()
        .find(|hint| hint.rule == Rule::OurServerError)
        .expect("fires");

    assert_eq!(hint.text, "GET /resolve answered 503 on our server.");
}

#[test]
fn our_slowness_compares_the_root_span_with_the_client_timeout() {
    let timed_out = bundle(
        vec![attempt(
            T1,
            "aaaaaaaaaaaaaaa1",
            "direct",
            0,
            Some("timeout after 10s"),
            AT,
        )],
        vec![],
    );
    let slow = trace(
        T1,
        vec![
            span(
                "api-main",
                "main",
                "GET /x",
                "1111111111111111",
                Some("aaaaaaaaaaaaaaa1"),
                0,
                12_000,
                false,
                &[],
            ),
            span(
                "api-main",
                "main",
                "upstream soundcloud",
                "2222222222222222",
                Some("1111111111111111"),
                100,
                11_000,
                false,
                &[],
            ),
        ],
    );
    fires(Rule::OurSlowness, &timed_out, &[slow], &[]);
    let quick = trace(
        T1,
        vec![span(
            "api-main",
            "main",
            "GET /x",
            "1111111111111111",
            Some("aaaaaaaaaaaaaaa1"),
            0,
            900,
            false,
            &[],
        )],
    );
    quiet(Rule::OurSlowness, &timed_out, &[quick], &[]);
}

#[test]
fn tier_missing_reads_the_stream_outcome() {
    let served = bundle(
        vec![attempt(T1, "aaaaaaaaaaaaaaa1", "direct", 200, None, AT)],
        vec![],
    );
    let missed = trace(
        T1,
        vec![span(
            "streaming-main",
            "main",
            "GET /stream/{id}",
            "1111111111111111",
            Some("aaaaaaaaaaaaaaa1"),
            0,
            900,
            false,
            &[("scd.outcome", "missed_hq")],
        )],
    );
    fires(Rule::TierMissing, &served, &[missed], &[]);
    let fine = trace(
        T1,
        vec![span(
            "streaming-main",
            "main",
            "GET /stream/{id}",
            "1111111111111111",
            Some("aaaaaaaaaaaaaaa1"),
            0,
            900,
            false,
            &[("scd.outcome", "served")],
        )],
    );
    quiet(Rule::TierMissing, &served, &[fine], &[]);
}

#[test]
fn region_blocked_reads_451_and_region_words() {
    let blocked = bundle(
        vec![attempt(T1, "aaaaaaaaaaaaaaa1", "direct", 451, None, AT)],
        vec![],
    );
    fires(Rule::RegionBlocked, &blocked, &[], &[]);
    let said = bundle(
        vec![attempt(
            T1,
            "aaaaaaaaaaaaaaa1",
            "direct",
            404,
            Some("not available in your country"),
            AT,
        )],
        vec![],
    );
    fires(Rule::RegionBlocked, &said, &[], &[]);
    let plain = bundle(
        vec![attempt(
            T1,
            "aaaaaaaaaaaaaaa1",
            "direct",
            404,
            Some("not found"),
            AT,
        )],
        vec![],
    );
    quiet(Rule::RegionBlocked, &plain, &[], &[]);
}

#[test]
fn session_expired_and_busy_follow_the_status() {
    let expired = bundle(
        vec![attempt(T1, "aaaaaaaaaaaaaaa1", "direct", 401, None, AT)],
        vec![],
    );
    fires(Rule::SessionExpired, &expired, &[], &[]);
    quiet(Rule::Busy, &expired, &[], &[]);
    let busy = bundle(
        vec![attempt(T1, "aaaaaaaaaaaaaaa1", "direct", 429, None, AT)],
        vec![],
    );
    fires(Rule::Busy, &busy, &[], &[]);
    quiet(Rule::SessionExpired, &busy, &[], &[]);
}

#[test]
fn issue_stats_drive_widespread_local_only_and_new_in_version() {
    let any = bundle(vec![], vec![error("0123456789abcdef")]);

    let wide = [error_row(Some(stats(30, 25, "8.4.20")))];
    fires(Rule::Widespread, &any, &[], &wide);
    quiet(Rule::LocalOnly, &any, &[], &wide);
    quiet(Rule::NewInVersion, &any, &[], &wide);

    let narrow = [error_row(Some(stats(1, 1, "8.4.25")))];
    quiet(Rule::Widespread, &any, &[], &narrow);
    fires(Rule::LocalOnly, &any, &[], &narrow);
    fires(Rule::NewInVersion, &any, &[], &narrow);

    let unknown = [error_row(None)];
    assert!(rules(&any, &[], &unknown).is_empty());
}

#[test]
fn hints_point_at_timeline_entries() {
    let failing = bundle(
        vec![
            attempt(
                T1,
                "aaaaaaaaaaaaaaa1",
                "r2",
                0,
                Some("connect timeout after 10s"),
                AT,
            ),
            attempt(
                T1,
                "aaaaaaaaaaaaaaa2",
                "direct",
                200,
                None,
                "2026-10-29T10:00:11.000Z",
            ),
        ],
        vec![],
    );
    let built = timeline::build(&failing, &[]);
    let hints = run(&Facts {
        bundle: &failing,
        timeline: &built,
        traces: &[],
        errors: &[],
    });
    let relay = hints
        .iter()
        .find(|hint| hint.rule == Rule::RelayBlocked)
        .expect("relay hint");

    assert_eq!(relay.evidence.timeline, vec![0, 1]);
}

#[test]
fn identical_neighbours_fold_and_server_time_follows_the_client_clock() {
    let repeated = bundle(
        vec![
            attempt(
                T2,
                "bbbbbbbbbbbbbbb1",
                "r2",
                0,
                Some("connect"),
                "2026-10-29T10:00:00.000Z",
            ),
            attempt(
                T2,
                "bbbbbbbbbbbbbbb2",
                "r2",
                0,
                Some("connect"),
                "2026-10-29T10:00:01.000Z",
            ),
            attempt(
                T1,
                "aaaaaaaaaaaaaaa1",
                "direct",
                200,
                None,
                "2026-10-29T10:00:05.000Z",
            ),
        ],
        vec![],
    );
    let seen = trace(
        T1,
        vec![span(
            "api-main",
            "main",
            "GET /x",
            "1111111111111111",
            Some("aaaaaaaaaaaaaaa1"),
            5_000 + 3_000,
            40,
            false,
            &[],
        )],
    );
    let built = timeline::build(&repeated, &[seen]);

    assert_eq!(built.entries.len(), 3);
    assert_eq!(built.entries[0].count, 2);
    assert_eq!(built.attempts[0], built.attempts[1]);
    assert_eq!(built.offset_ms, Some(3_000));
    assert_eq!(built.entries[2].at, "2026-10-29T10:00:05.000Z");
}

#[test]
fn a_public_report_names_no_ids_addresses_or_internal_hosts() {
    let leaky = bundle(
        vec![
            attempt(
                T1,
                "aaaaaaaaaaaaaaa1",
                "r2",
                0,
                Some(
                    "connect to 188.165.221.195:443 and [2001:db8::7]:443 failed, api-star.scnative.dev, haproxy:3000, jaeger:4317 a1b2c3d4e5f6 mail me@example.com",
                ),
                AT,
            ),
            attempt(T1, "aaaaaaaaaaaaaaa2", "direct", 200, None, AT),
        ],
        vec![error("0123456789abcdef")],
    );
    let mut root = span(
        "streaming-star",
        "star",
        "GET /stream/{id}",
        "1111111111111111",
        Some("aaaaaaaaaaaaaaa2"),
        0,
        900,
        true,
        &[("scd.outcome", "exhausted"), ("scd.host", "star")],
    );
    root.message = Some("upstream 10.1.2.3:8080 refused".to_owned());
    let child = span(
        "streaming-star",
        "star",
        "source relay_track",
        "2222222222222222",
        Some("1111111111111111"),
        5,
        300,
        true,
        &[
            ("scd.source", "relay_track"),
            ("error.message", "relay_track failed via 172.18.0.5"),
        ],
    );
    let upstream = span(
        "api-star",
        "star",
        "upstream apiv2_tracks",
        "3333333333333333",
        Some("1111111111111111"),
        5,
        300,
        true,
        &[("scd.upstream", "apiv2_tracks"), ("scd.route", "relay")],
    );
    let traces = vec![trace(T1, vec![root, child, upstream])];
    let timeline = timeline::build(&leaky, &traces);
    let mut errors = vec![error_row(Some(stats(1, 1, "8.4.25")))];
    errors[0].message = format!("trace {T1} install a1b2c3d4e5f6 at ::1 and 2001:db8::9");
    let hints = run(&Facts {
        bundle: &leaky,
        timeline: &timeline,
        traces: &traces,
        errors: &errors,
    });
    let mut report = Report {
        header: Header {
            view: View::Internal,
            code: "SCD-7F3K92QA".to_owned(),
            created: leaky.at.clone(),
            app: ReportApp::from_app(&leaky.app),
            session: leaky.session.clone(),
        },
        counts: Counts::default(),
        offset_ms: timeline.offset_ms,
        timeline: timeline.entries,
        errors,
        hints,
        missing: Missing {
            traces: 1,
            trace_ids: vec![T2.to_owned()],
            sourcemaps: SourcemapState::Missing,
        },
    };

    public::redact(&mut report);
    let text = serde_json::to_string(&report).expect("json");

    for needle in [
        T1,
        T2,
        "a1b2c3d4e5f6",
        "188.165.221.195",
        "2001:db8",
        "10.1.2.3",
        "172.18.0.5",
        "::1",
        "scnative",
        "haproxy",
        "jaeger",
        "relay_track",
        "apiv2_tracks",
        "example.com",
    ] {
        assert!(
            !text.contains(needle),
            "public report leaks {needle}: {text}"
        );
    }
    assert!(text.contains("relay r2"));
    assert!(text.contains("star streaming"));
    assert!(text.contains("source relay"));
    assert!(text.contains("\"view\":\"public\""));
}
