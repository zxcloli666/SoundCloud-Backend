INSERT INTO playlist_reconcile_runs (
    playlist_urn,
    reconcile_generation,
    job_id,
    job_generation,
    captured_baseline_generation,
    captured_through_operation_sequence
)
VALUES ($1, $2, $3, $4, $5, $6)
RETURNING id
