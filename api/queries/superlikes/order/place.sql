INSERT INTO user_superlike_ranks (user_id, sc_track_id, superlike_created_at, rank)
SELECT $1, placed.sc_track_id, placed.created_at, placed.rank
FROM unnest($2::text[], $3::timestamptz[], $4::int8[]) AS placed(sc_track_id, created_at, rank)
ON CONFLICT (user_id, sc_track_id) DO UPDATE
SET superlike_created_at = EXCLUDED.superlike_created_at, rank = EXCLUDED.rank
WHERE (user_superlike_ranks.superlike_created_at, user_superlike_ranks.rank)
      IS DISTINCT FROM (EXCLUDED.superlike_created_at, EXCLUDED.rank)
