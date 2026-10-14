SELECT min(rank) AS rank
FROM user_like_ranks
WHERE user_id = $1 AND rank > $2 AND sc_track_id <> $3
