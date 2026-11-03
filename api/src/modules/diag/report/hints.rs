use backend_contracts::diag::{Attempt, Bundle};
use backend_contracts::diag_report::{Evidence, Hint, ReportError, Rule};

use super::super::jaeger::{SpanView, TraceView};
use super::timeline::Timeline;

const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const WIDESPREAD_INSTALLS: i64 = 20;

pub struct Facts<'a> {
    pub bundle: &'a Bundle,
    pub timeline: &'a Timeline,
    pub traces: &'a [TraceView],
    pub errors: &'a [ReportError],
}

type RuleFn = fn(&Facts) -> Option<Hint>;

const RULES: [RuleFn; 13] = [
    relay_blocked,
    origin_blocked,
    all_routes_down,
    never_reached,
    our_server_error,
    our_slowness,
    tier_missing,
    region_blocked,
    session_expired,
    busy,
    widespread,
    local_only,
    new_in_version,
];

pub fn run(facts: &Facts) -> Vec<Hint> {
    RULES.iter().filter_map(|rule| rule(facts)).collect()
}

fn relay_blocked(facts: &Facts) -> Option<Hint> {
    let mut hops: Vec<&str> = Vec::new();
    let mut evidence = Vec::new();
    for group in facts.groups() {
        let Some(worked) = group.iter().find(|(_, attempt)| succeeded(attempt)) else {
            continue;
        };
        for (index, attempt) in &group {
            if attempt.status == 0 && attempt.hop != "direct" && attempt.hop != worked.1.hop {
                hops.push(&attempt.hop);
                evidence.push(facts.entry(*index));
                evidence.push(facts.entry(worked.0));
            }
        }
    }
    hops.sort_unstable();
    hops.dedup();
    (!hops.is_empty()).then(|| {
        facts.hint(
            Rule::RelayBlocked,
            format!(
                "{} failed to connect while another route worked.",
                hops.iter()
                    .map(|hop| hop_name(hop))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            evidence,
        )
    })
}

fn hop_name(hop: &str) -> String {
    match hop {
        "relay" => "A relay".to_owned(),
        "pro" => "A pro relay".to_owned(),
        _ if hop.starts_with('p') => format!("Pro relay {hop}"),
        _ => format!("Relay {hop}"),
    }
}

fn origin_blocked(facts: &Facts) -> Option<Hint> {
    let mut evidence = Vec::new();
    for group in facts.groups() {
        let Some(worked) = group
            .iter()
            .find(|(_, attempt)| attempt.hop != "direct" && succeeded(attempt))
        else {
            continue;
        };
        for (index, attempt) in &group {
            if attempt.status == 0 && attempt.hop == "direct" {
                evidence.push(facts.entry(*index));
                evidence.push(facts.entry(worked.0));
            }
        }
    }
    (!evidence.is_empty()).then(|| {
        facts.hint(
            Rule::OriginBlocked,
            "The direct connection failed while a relay worked: the origin is probably blocked for this client.".to_owned(),
            evidence,
        )
    })
}

fn all_routes_down(facts: &Facts) -> Option<Hint> {
    let mut evidence = Vec::new();
    for group in facts.groups() {
        let mut hops: Vec<&str> = group
            .iter()
            .map(|(_, attempt)| attempt.hop.as_str())
            .collect();
        hops.sort_unstable();
        hops.dedup();
        if hops.len() >= 2 && group.iter().all(|(_, attempt)| attempt.status == 0) {
            evidence.extend(group.iter().map(|(index, _)| facts.entry(*index)));
        }
    }
    (!evidence.is_empty()).then(|| {
        facts.hint(
            Rule::AllRoutesDown,
            "Every route failed without any answer: DNS or no network on the client.".to_owned(),
            evidence,
        )
    })
}

fn never_reached(facts: &Facts) -> Option<Hint> {
    let evidence: Vec<usize> = facts
        .bundle
        .attempts
        .iter()
        .enumerate()
        .filter(|(_, attempt)| failed_hard(attempt) && !facts.has_server_span(attempt))
        .map(|(index, _)| facts.entry(index))
        .collect();
    (!evidence.is_empty()).then(|| {
        facts.hint(
            Rule::NeverReached,
            "The request failed and left no trace on our servers: it never arrived or died on the way.".to_owned(),
            evidence,
        )
    })
}

fn our_server_error(facts: &Facts) -> Option<Hint> {
    let mut evidence = Vec::new();
    let mut request = None;
    let mut message = None;
    for attempt in &facts.bundle.attempts {
        let Some((trace, root)) = facts.root_span(attempt) else {
            continue;
        };
        if !failed_hard(attempt) || root.status != "error" {
            continue;
        }
        let status = root
            .attrs
            .get("http.response.status_code")
            .and_then(|value| value.as_u64())
            .unwrap_or(u64::from(attempt.status));
        if request.is_none() {
            request = Some(format!("{} answered {status} on our server.", root.name));
        }
        if message.is_none() {
            message = span_message(root);
        }
        evidence.extend(
            facts
                .timeline
                .spans
                .get(&(trace.trace.clone(), root.span.clone())),
        );
    }
    for trace in facts.traces {
        let failed = facts
            .bundle
            .attempts
            .iter()
            .any(|attempt| attempt.trace == trace.trace && failed_hard(attempt));
        if !failed {
            continue;
        }
        for span in trace.spans.iter().filter(|span| span.status == "error") {
            if message.is_none() {
                message = span_message(span);
            }
            evidence.extend(
                facts
                    .timeline
                    .spans
                    .get(&(trace.trace.clone(), span.span.clone())),
            );
        }
    }
    (!evidence.is_empty()).then(|| {
        let mut text = request.unwrap_or_else(|| "Our server reported an error.".to_owned());
        if let Some(message) = message {
            text = format!("{text} Error: {message}");
        }
        facts.hint(Rule::OurServerError, text, evidence)
    })
}

fn our_slowness(facts: &Facts) -> Option<Hint> {
    let mut evidence = Vec::new();
    let mut text = None;
    for attempt in &facts.bundle.attempts {
        let Some((trace, root)) = facts.root_span(attempt) else {
            continue;
        };
        let timeout = client_timeout_ms(attempt);
        if root.ms <= timeout {
            continue;
        }
        let slowest = trace
            .spans
            .iter()
            .filter(|span| span.parent.as_deref() == Some(root.span.as_str()))
            .max_by_key(|span| span.ms);
        evidence.extend(
            facts
                .timeline
                .spans
                .get(&(trace.trace.clone(), root.span.clone())),
        );
        if let Some(slowest) = slowest {
            evidence.extend(
                facts
                    .timeline
                    .spans
                    .get(&(trace.trace.clone(), slowest.span.clone())),
            );
        }
        if text.is_none() {
            text = Some(match slowest {
                Some(slowest) => format!(
                    "Our server took {} ms, longer than the client timeout of {} ms; the slowest part was {} ({} ms).",
                    root.ms, timeout, slowest.name, slowest.ms
                ),
                None => format!(
                    "Our server took {} ms, longer than the client timeout of {} ms.",
                    root.ms, timeout
                ),
            });
        }
    }
    text.map(|text| facts.hint(Rule::OurSlowness, text, evidence))
}

fn tier_missing(facts: &Facts) -> Option<Hint> {
    let mut tiers: Vec<&str> = Vec::new();
    let mut evidence = Vec::new();
    for trace in facts.traces {
        for span in &trace.spans {
            let tier = match span
                .attrs
                .get("scd.outcome")
                .and_then(|value| value.as_str())
            {
                Some("missed_hq") => "hq",
                Some("missed_uq") => "uq",
                _ => continue,
            };
            tiers.push(tier);
            evidence.extend(
                facts
                    .timeline
                    .spans
                    .get(&(trace.trace.clone(), span.span.clone())),
            );
        }
    }
    tiers.sort_unstable();
    tiers.dedup();
    (!tiers.is_empty()).then(|| {
        facts.hint(
            Rule::TierMissing,
            format!(
                "Quality {} was not available for the track, playback stepped down.",
                tiers.join(", ")
            ),
            evidence,
        )
    })
}

fn region_blocked(facts: &Facts) -> Option<Hint> {
    let mut evidence = Vec::new();
    for (index, attempt) in facts.bundle.attempts.iter().enumerate() {
        let said = attempt.error.as_deref().is_some_and(mentions_region);
        if attempt.status == 451 || said {
            evidence.push(facts.entry(index));
        }
    }
    for trace in facts.traces {
        for span in &trace.spans {
            let said = span_message(span).as_deref().is_some_and(mentions_region);
            if said {
                evidence.extend(
                    facts
                        .timeline
                        .spans
                        .get(&(trace.trace.clone(), span.span.clone())),
                );
            }
        }
    }
    (!evidence.is_empty()).then(|| {
        facts.hint(
            Rule::RegionBlocked,
            "The track looks blocked in the country of this client.".to_owned(),
            evidence,
        )
    })
}

fn session_expired(facts: &Facts) -> Option<Hint> {
    let evidence = facts.attempts_with_status(401);
    (!evidence.is_empty()).then(|| {
        facts.hint(
            Rule::SessionExpired,
            "The server answered 401: the session expired, signing in again helps.".to_owned(),
            evidence,
        )
    })
}

fn busy(facts: &Facts) -> Option<Hint> {
    let evidence = facts.attempts_with_status(429);
    (!evidence.is_empty()).then(|| {
        facts.hint(
            Rule::Busy,
            "The server answered 429: too many requests from this client or the service is busy."
                .to_owned(),
            evidence,
        )
    })
}

fn widespread(facts: &Facts) -> Option<Hint> {
    let errors = facts.errors_where(|error| {
        error
            .issue
            .as_ref()
            .is_some_and(|issue| issue.installs_24h >= WIDESPREAD_INSTALLS)
    });
    (!errors.is_empty()).then(|| {
        facts.error_hint(
            Rule::Widespread,
            format!(
                "The same error hit {WIDESPREAD_INSTALLS} or more installs in the last 24 hours."
            ),
            errors,
        )
    })
}

fn local_only(facts: &Facts) -> Option<Hint> {
    let errors = facts.errors_where(|error| {
        error
            .issue
            .as_ref()
            .is_some_and(|issue| issue.installs == 1)
    });
    (!errors.is_empty()).then(|| {
        facts.error_hint(
            Rule::LocalOnly,
            "Only this install has this error.".to_owned(),
            errors,
        )
    })
}

fn new_in_version(facts: &Facts) -> Option<Hint> {
    let version = &facts.bundle.app.version;
    let errors = facts.errors_where(|error| {
        error
            .issue
            .as_ref()
            .is_some_and(|issue| &issue.first_version == version)
    });
    (!errors.is_empty()).then(|| {
        facts.error_hint(
            Rule::NewInVersion,
            format!("This error first appeared in version {version}."),
            errors,
        )
    })
}

impl Facts<'_> {
    fn groups(&self) -> Vec<Vec<(usize, &Attempt)>> {
        let mut groups: Vec<Vec<(usize, &Attempt)>> = Vec::new();
        for (index, attempt) in self.bundle.attempts.iter().enumerate() {
            match groups
                .iter_mut()
                .find(|group| group[0].1.trace == attempt.trace)
            {
                Some(group) => group.push((index, attempt)),
                None => groups.push(vec![(index, attempt)]),
            }
        }
        groups
    }

    fn root_span(&self, attempt: &Attempt) -> Option<(&TraceView, &SpanView)> {
        let trace = self
            .traces
            .iter()
            .find(|trace| trace.trace == attempt.trace)?;
        let root = trace
            .spans
            .iter()
            .find(|span| span.parent.as_deref() == Some(attempt.span.as_str()))?;
        Some((trace, root))
    }

    fn has_server_span(&self, attempt: &Attempt) -> bool {
        self.root_span(attempt).is_some()
    }

    fn attempts_with_status(&self, status: u16) -> Vec<usize> {
        self.bundle
            .attempts
            .iter()
            .enumerate()
            .filter(|(_, attempt)| attempt.status == status)
            .map(|(index, _)| self.entry(index))
            .collect()
    }

    fn errors_where(&self, test: impl Fn(&ReportError) -> bool) -> Vec<usize> {
        self.errors
            .iter()
            .enumerate()
            .filter(|(_, error)| test(error))
            .map(|(index, _)| index)
            .collect()
    }

    fn entry(&self, attempt: usize) -> usize {
        self.timeline.attempts[attempt]
    }

    fn hint(&self, rule: Rule, text: String, entries: Vec<usize>) -> Hint {
        let mut timeline: Vec<u32> = entries.into_iter().map(|index| index as u32).collect();
        timeline.sort_unstable();
        timeline.dedup();
        Hint {
            rule,
            text,
            evidence: Evidence {
                timeline,
                errors: Vec::new(),
            },
        }
    }

    fn error_hint(&self, rule: Rule, text: String, errors: Vec<usize>) -> Hint {
        Hint {
            rule,
            text,
            evidence: Evidence {
                timeline: Vec::new(),
                errors: errors.into_iter().map(|index| index as u32).collect(),
            },
        }
    }
}

fn succeeded(attempt: &Attempt) -> bool {
    (200..400).contains(&attempt.status) && attempt.error.is_none()
}

fn failed_hard(attempt: &Attempt) -> bool {
    attempt.status == 0 || attempt.status >= 500 || attempt.error.is_some()
}

fn span_message(span: &SpanView) -> Option<String> {
    span.message.clone().or_else(|| {
        span.attrs
            .get("error.message")
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    })
}

fn mentions_region(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    ["region", "country", "geo", "blocked in"]
        .iter()
        .any(|word| text.contains(word))
}

fn client_timeout_ms(attempt: &Attempt) -> u64 {
    let seconds = attempt.error.as_deref().and_then(|error| {
        let (_, rest) = error.split_once("timeout after ")?;
        rest.split('s').next()?.trim().parse::<u64>().ok()
    });
    seconds.map_or(DEFAULT_TIMEOUT_MS, |seconds| seconds * 1000)
}
