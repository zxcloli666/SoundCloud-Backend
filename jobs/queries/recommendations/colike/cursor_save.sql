INSERT INTO recommendation_colike_cursor (singleton, next_a_id, updated_at)
VALUES (true, $1, now())
ON CONFLICT (singleton) DO UPDATE
SET next_a_id = excluded.next_a_id,
    updated_at = excluded.updated_at
