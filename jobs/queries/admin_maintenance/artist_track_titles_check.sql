WITH batch AS MATERIALIZED (
    SELECT track.id,
           track.primary_artist_id,
           track.title_normalized COLLATE "C" AS title_normalized,
           track.superseded_by IS NULL AND track.title_normalized IS NOT NULL AS titled
    FROM tracks AS track
    WHERE track.id > $1
    ORDER BY track.id
    LIMIT $2
), bounds AS MATERIALIZED (
    SELECT last.id AS last,
           CASE
               WHEN (SELECT count(*) FROM batch) < $2 THEN 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid
               ELSE last.id
           END AS upper,
           (SELECT count(*) FROM batch) AS scanned
    FROM (SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS id) AS last
), expected AS MATERIALIZED (
    SELECT batch.primary_artist_id AS artist_id, batch.title_normalized, batch.id AS track_id
    FROM batch
    WHERE batch.titled
      AND batch.primary_artist_id IS NOT NULL
    UNION
    SELECT credit.artist_id, batch.title_normalized, batch.id
    FROM batch
    JOIN track_artists AS credit ON credit.track_id = batch.id AND credit.role = 'primary'
    WHERE batch.titled
), stored AS MATERIALIZED (
    SELECT title.artist_id, title.title_normalized, title.track_id
    FROM bounds
    JOIN artist_track_titles AS title ON title.track_id > $1 AND title.track_id <= bounds.upper
)
SELECT bounds.last AS "last?",
       bounds.scanned AS "scanned!",
       ARRAY(
           SELECT DISTINCT coalesce(expected.track_id, stored.track_id)
           FROM expected
           FULL JOIN stored ON stored.artist_id = expected.artist_id
                           AND stored.title_normalized = expected.title_normalized
                           AND stored.track_id = expected.track_id
           WHERE expected.track_id IS NULL
              OR stored.track_id IS NULL
           ORDER BY 1
       ) AS "mismatched!"
FROM bounds
