UPDATE artists AS artist
SET popularity_score = scored.popularity_score
FROM (
    SELECT candidate.id,
           round(COALESCE(
               LEAST(
                   1.0::real,
                   ln(greatest(totals.score, 0) + 1)::real
                       / NULLIF(ln(maximum.score + 1)::real, 0)
               ),
               0
           )::numeric, 4)::real AS popularity_score
    FROM artists AS candidate
    CROSS JOIN artist_play_maximum AS maximum
    LEFT JOIN artist_play_totals AS totals ON totals.artist_id = candidate.id
    WHERE candidate.merged_into IS NULL
      AND candidate.id BETWEEN $1 AND $2
) AS scored
WHERE artist.id = scored.id
  AND artist.id BETWEEN $1 AND $2
  AND artist.popularity_score IS DISTINCT FROM scored.popularity_score
