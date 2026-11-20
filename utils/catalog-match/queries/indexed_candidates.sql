WITH keyed AS MATERIALIZED (
    SELECT keys.track_id
    FROM track_work_keys AS keys
    WHERE keys.work_key = ANY ($2)
      AND keys.primary_artist_id = $1
    UNION
    SELECT keys.track_id
    FROM track_work_keys AS keys
    CROSS JOIN LATERAL (SELECT 1
                        FROM track_artists AS credit
                        WHERE credit.track_id = keys.track_id
                          AND credit.artist_id = $1
                          AND credit.role = 'primary'
                        LIMIT 1) AS credited
    WHERE keys.work_key = ANY ($2)
    UNION
    SELECT alias.track_id
    FROM track_work_aliases AS alias
    JOIN track_work_keys AS keys ON keys.track_id = alias.track_id AND keys.primary_artist_id = $1
    WHERE alias.alias_key = ANY ($2)
    UNION
    SELECT alias.track_id
    FROM track_work_aliases AS alias
    CROSS JOIN LATERAL (SELECT 1
                        FROM track_artists AS credit
                        WHERE credit.track_id = alias.track_id
                          AND credit.artist_id = $1
                          AND credit.role = 'primary'
                        LIMIT 1) AS credited
    WHERE alias.alias_key = ANY ($2)
), titled AS MATERIALIZED (
    SELECT title.track_id,
           min(CASE WHEN title.title_normalized = $3 THEN 1 ELSE 2 END) AS rank
    FROM artist_track_titles AS title
    WHERE title.artist_id = $1
      AND title.title_normalized >= $4
      AND title.title_normalized < $5
    GROUP BY title.track_id
), ranked AS MATERIALIZED (
    SELECT candidate.track_id, min(candidate.rank) AS rank
    FROM (SELECT keyed.track_id, 0 AS rank
          FROM keyed
          UNION ALL
          SELECT titled.track_id, titled.rank
          FROM titled) AS candidate
    GROUP BY candidate.track_id
)
SELECT track.id,
       track.sc_track_id,
       COALESCE(track.title, '') AS "title!"
FROM ranked
JOIN tracks AS track ON track.id = ranked.track_id
WHERE track.superseded_by IS NULL
ORDER BY ranked.rank, track.id
LIMIT $6
