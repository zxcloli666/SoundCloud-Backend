UPDATE playlist_membership_state
SET latest_observation_id = $2,
    sync_status = $3,
    conflict_code = $4,
    reconcile_failure_streak = least(reconcile_failure_streak + 1, 12),
    next_reconcile_at = greatest(
        $5,
        clock_timestamp()
            + make_interval(
                  secs => least(
                      120 * power(2, least(reconcile_failure_streak, 9))::double precision,
                      86400
                  )
              )
    ),
    last_error = $6,
    updated_at = clock_timestamp()
WHERE playlist_urn = $1
  AND reconcile_generation = $7
