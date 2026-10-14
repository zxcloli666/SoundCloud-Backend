WITH ordered AS (
    SELECT sc_track_id, row_number() OVER (ORDER BY rank, sc_track_id) AS position
    FROM user_like_ranks
    WHERE user_id = $1
)
UPDATE user_like_ranks ranked
SET rank = ordered.position * $2
FROM ordered
WHERE ranked.user_id = $1
  AND ranked.sc_track_id = ordered.sc_track_id
  AND ranked.rank <> ordered.position * $2
