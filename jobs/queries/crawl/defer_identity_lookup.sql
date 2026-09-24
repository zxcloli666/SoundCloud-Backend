UPDATE artists
SET genius_next_run_at = now() + ($2 * interval '1 day'),
    genius_locked_at   = NULL
WHERE id = $1
