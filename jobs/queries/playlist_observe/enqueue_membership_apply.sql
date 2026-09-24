INSERT INTO sync_queue (user_id, action_type, target_urn, payload, next_run_at)
SELECT $2,
       'playlist_membership',
       state.playlist_urn,
       jsonb_build_object(
           'tracks', to_jsonb($3::text[]),
           'fingerprint', encode($4::bytea, 'hex'),
           'reconcile_generation', $5::bigint
       ),
       clock_timestamp()
FROM playlist_membership_state AS state
WHERE state.playlist_urn = $1
  AND state.sync_status = 'shadow_ready'
  AND state.reconcile_generation = $5::bigint
  AND state.remote_apply_fingerprint IS DISTINCT FROM $4::bytea
ON CONFLICT (user_id, action_type, target_urn)
    WHERE action_type <> 'comment'
DO UPDATE SET
    payload = EXCLUDED.payload,
    generation = sync_queue.generation + 1,
    remote_attempted_generation = NULL,
    remote_completed_generation = NULL,
    remote_result = NULL,
    retry_count = 0,
    last_error = NULL,
    next_run_at = clock_timestamp(),
    dead = false,
    failed_at = NULL
