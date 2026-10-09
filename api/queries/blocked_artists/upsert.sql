INSERT INTO user_blocked_artists (sc_user_id, kind, target_id, name, avatar_url, sc_user_ids)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (sc_user_id, kind, target_id) DO UPDATE
    SET name        = EXCLUDED.name,
        avatar_url  = EXCLUDED.avatar_url,
        sc_user_ids = EXCLUDED.sc_user_ids
RETURNING kind, target_id, name, avatar_url, sc_user_ids, created_at
