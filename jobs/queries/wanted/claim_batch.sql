WITH picked AS (SELECT id
                FROM wanted_tracks
                WHERE status = 'wanted'
                  AND track_id IS NULL
                  AND resolve_next_run_at <= now()
                  AND (resolve_locked_at IS NULL
                    OR resolve_locked_at < now() - ($1 * interval '1 second'))
                ORDER BY resolve_next_run_at
                LIMIT $2 FOR UPDATE SKIP LOCKED)
UPDATE wanted_tracks AS wanted
SET resolve_locked_at = now(),
    resolve_attempts  = wanted.resolve_attempts + 1
FROM picked
WHERE wanted.id = picked.id
RETURNING wanted.id
