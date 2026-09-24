UPDATE artists
SET account_walk_locked_at = NULL
WHERE id = $1
