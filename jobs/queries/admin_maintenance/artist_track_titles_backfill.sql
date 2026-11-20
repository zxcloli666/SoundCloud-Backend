WITH batch AS MATERIALIZED (
    SELECT track.id,
           track.primary_artist_id,
           track.title_normalized COLLATE "C" AS title_normalized,
           track.superseded_by IS NULL AND track.title_normalized IS NOT NULL AS titled
    FROM tracks AS track
    WHERE track.id > $1
    ORDER BY track.id
    LIMIT $2
), wanted AS MATERIALIZED (
    SELECT batch.primary_artist_id AS artist_id, batch.title_normalized, batch.id AS track_id
    FROM batch
    WHERE batch.titled
      AND batch.primary_artist_id IS NOT NULL
    UNION
    SELECT credit.artist_id, batch.title_normalized, batch.id
    FROM batch
    JOIN track_artists AS credit ON credit.track_id = batch.id AND credit.role = 'primary'
    WHERE batch.titled
), written AS (
    INSERT INTO artist_track_titles (artist_id, title_normalized, track_id)
    SELECT wanted.artist_id, wanted.title_normalized, wanted.track_id
    FROM wanted
    ORDER BY wanted.artist_id, wanted.title_normalized, wanted.track_id
    ON CONFLICT DO NOTHING
    RETURNING track_id
)
SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS "last?",
       (SELECT count(*) FROM batch) AS "scanned!",
       (SELECT count(*) FROM written) AS "inserted!"
