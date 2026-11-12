CREATE TEMP TABLE artist_play_maximum AS
SELECT greatest(max(score), 1)::bigint AS score
FROM artist_play_totals
