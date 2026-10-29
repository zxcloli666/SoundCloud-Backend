mod frames;
mod hints;
mod public;
mod scrub;
mod select;
mod sourcemaps;
mod timeline;

use std::collections::HashMap;

use backend_contracts::diag::Bundle;
use backend_contracts::diag_report::{
    Counts, Header, Missing, Report, ReportApp, ReportError, View,
};
use futures::StreamExt;

use super::DiagService;
use super::code;
use super::jaeger::TraceView;
use crate::error::AppError;

pub use sourcemaps::Sourcemaps;

const TRACE_LOOKUPS_AT_ONCE: usize = 4;
const ERRORS_WITH_STATS: usize = 10;

pub async fn build(
    service: &DiagService,
    code: &str,
    view: View,
) -> Result<Option<Report>, AppError> {
    let (Some(code), Some(stored)) = (code::normalize(code), service.bundle(code).await?) else {
        return Ok(None);
    };
    let bundle: Bundle = serde_json::from_value(stored)
        .map_err(|error| AppError::internal(format!("stored bundle: {error}")))?;

    let wanted = select::wanted(&bundle);
    let (traces, lost) = fetch_traces(service, &wanted).await;
    let timeline = timeline::build(&bundle, &traces);
    let mut errors = report_errors(service, &bundle).await;
    let sourcemaps = service
        .sourcemaps()
        .apply(&bundle.app.version, &mut errors)
        .await;
    let hints = hints::run(&hints::Facts {
        bundle: &bundle,
        timeline: &timeline,
        traces: &traces,
        errors: &errors,
    });

    let failed_attempts = bundle
        .attempts
        .iter()
        .filter(|attempt| attempt.status == 0 || attempt.status >= 400 || attempt.error.is_some())
        .count();
    let mut report = Report {
        header: Header {
            view: View::Internal,
            code,
            created: bundle.at.clone(),
            app: ReportApp::from_app(&bundle.app),
            session: bundle.session.clone(),
        },
        counts: Counts {
            attempts: bundle.attempts.len() as u32,
            failed_attempts: failed_attempts as u32,
            errors: bundle.errors.len() as u32,
            traces_wanted: wanted.len() as u32,
            traces_found: traces.len() as u32,
            server_spans: traces.iter().map(|trace| trace.spans.len() as u32).sum(),
        },
        offset_ms: timeline.offset_ms,
        timeline: timeline.entries,
        errors,
        hints,
        missing: Missing {
            traces: lost.len() as u32,
            trace_ids: lost,
            sourcemaps,
        },
    };
    if view == View::Public {
        public::redact(&mut report);
    }
    Ok(Some(report))
}

async fn fetch_traces(service: &DiagService, wanted: &[String]) -> (Vec<TraceView>, Vec<String>) {
    let results: Vec<_> = futures::stream::iter(wanted.iter().cloned())
        .map(|trace| async move { service.trace(&trace).await })
        .buffered(TRACE_LOOKUPS_AT_ONCE)
        .collect()
        .await;
    let mut found = Vec::new();
    let mut lost = Vec::new();
    for (trace, result) in wanted.iter().zip(results) {
        match result {
            Ok(Some(view)) => found.push(view),
            Ok(None) | Err(_) => lost.push(trace.clone()),
        }
    }
    (found, lost)
}

async fn report_errors(service: &DiagService, bundle: &Bundle) -> Vec<ReportError> {
    let mut stats = HashMap::new();
    for error in &bundle.errors {
        if stats.len() == ERRORS_WITH_STATS || stats.contains_key(&error.fingerprint) {
            continue;
        }
        let found = service.issue_stats(&error.fingerprint).await.ok().flatten();
        stats.insert(error.fingerprint.clone(), found);
    }
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
            issue: stats.get(&error.fingerprint).cloned().flatten(),
            trace: error.trace.clone(),
        })
        .collect()
}

#[cfg(test)]
mod jaeger_view;
#[cfg(test)]
mod security_tests;
#[cfg(test)]
mod tests;
