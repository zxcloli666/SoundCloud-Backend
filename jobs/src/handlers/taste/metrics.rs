const EXPORT_SKIPPED: &str = "jobs_taste_export_skipped_total";
const EXPORTED: &str = "jobs_taste_exported_total";
const RESULTS: &str = "jobs_taste_results_total";
const USER_VECTORS: &str = "jobs_taste_user_vectors_written_total";
const ORPHANS: &str = "jobs_taste_orphan_objects_removed_total";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExportSkip {
    TooFewUsers,
    TooLarge,
    InFlight,
}

impl ExportSkip {
    pub(crate) const ALL: [Self; 3] = [Self::TooFewUsers, Self::TooLarge, Self::InFlight];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TooFewUsers => "too_few_users",
            Self::TooLarge => "too_large",
            Self::InFlight => "in_flight",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResultOutcome {
    Applied,
    Superseded,
    Duplicate,
    Rejected,
    Empty,
    Missing,
    Failed,
    Reopened,
    Invalid,
}

impl ResultOutcome {
    pub(crate) const ALL: [Self; 9] = [
        Self::Applied,
        Self::Superseded,
        Self::Duplicate,
        Self::Rejected,
        Self::Empty,
        Self::Missing,
        Self::Failed,
        Self::Reopened,
        Self::Invalid,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Superseded => "superseded",
            Self::Duplicate => "duplicate",
            Self::Rejected => "rejected",
            Self::Empty => "empty",
            Self::Missing => "missing",
            Self::Failed => "failed",
            Self::Reopened => "reopened",
            Self::Invalid => "invalid",
        }
    }
}

pub(crate) fn register_series_that_start_at_zero() {
    for reason in ExportSkip::ALL {
        metrics::counter!(EXPORT_SKIPPED, "reason" => reason.as_str()).increment(0);
    }
    for outcome in ResultOutcome::ALL {
        metrics::counter!(RESULTS, "outcome" => outcome.as_str()).increment(0);
    }
    metrics::counter!(EXPORTED).increment(0);
    metrics::counter!(USER_VECTORS).increment(0);
    metrics::counter!(ORPHANS).increment(0);
}

pub(crate) fn record_orphan_removed() {
    metrics::counter!(ORPHANS).increment(1);
}

pub(crate) fn record_export_skipped(reason: ExportSkip) {
    metrics::counter!(EXPORT_SKIPPED, "reason" => reason.as_str()).increment(1);
}

pub(crate) fn record_exported() {
    metrics::counter!(EXPORTED).increment(1);
}

pub(crate) fn record_result(outcome: ResultOutcome) {
    metrics::counter!(RESULTS, "outcome" => outcome.as_str()).increment(1);
}

pub(crate) fn record_user_vectors(count: usize) {
    metrics::counter!(USER_VECTORS).increment(u64::try_from(count).unwrap_or(u64::MAX));
}
