UPDATE sync_queue
SET lease_id = NULL,
    lease_generation = NULL,
    locked_at = NULL,
    dead = true,
    failed_at = coalesce(failed_at, now()),
    last_error = 'Playlist membership sync is blocked until remote rebase is available',
    next_run_at = 'infinity'
WHERE action_type = 'playlist_sync'
  AND (
      remote_completed_generation IS DISTINCT FROM generation
      OR remote_result IS NULL
  );

UPDATE sync_queue
SET lease_id = NULL,
    lease_generation = NULL,
    locked_at = NULL,
    dead = false,
    failed_at = NULL,
    last_error = NULL,
    next_run_at = now()
WHERE action_type = 'playlist_sync'
  AND remote_completed_generation = generation
  AND remote_result IS NOT NULL;
