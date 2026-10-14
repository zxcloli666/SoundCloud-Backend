INSERT INTO user_like_ranks (user_id, sc_track_id, like_created_at, rank)
VALUES ($1, $2, $3, $4)
ON CONFLICT (user_id, sc_track_id) DO UPDATE
SET like_created_at = EXCLUDED.like_created_at, rank = EXCLUDED.rank
