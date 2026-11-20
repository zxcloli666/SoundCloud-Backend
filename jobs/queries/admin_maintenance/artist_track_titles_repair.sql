WITH locked AS MATERIALIZED (
    SELECT track.id,
           track.primary_artist_id,
           track.title_normalized COLLATE "C" AS title_normalized,
           track.superseded_by IS NULL AND track.title_normalized IS NOT NULL AS titled
    FROM tracks AS track
    WHERE track.id = ANY ($1)
    ORDER BY track.id
    FOR SHARE
), current AS MATERIALIZED (
    SELECT locked.primary_artist_id AS artist_id, locked.title_normalized, locked.id AS track_id
    FROM locked
    WHERE locked.titled
      AND locked.primary_artist_id IS NOT NULL
    UNION
    SELECT credit.artist_id, locked.title_normalized, locked.id
    FROM locked
    JOIN track_artists AS credit ON credit.track_id = locked.id AND credit.role = 'primary'
    WHERE locked.titled
), removed AS (
    DELETE FROM artist_track_titles AS stored
    WHERE stored.track_id = ANY ($1)
      AND NOT EXISTS (
          SELECT 1
          FROM current
          WHERE current.artist_id = stored.artist_id
            AND current.title_normalized = stored.title_normalized
            AND current.track_id = stored.track_id
      )
    RETURNING stored.track_id
), written AS (
    INSERT INTO artist_track_titles (artist_id, title_normalized, track_id)
    SELECT current.artist_id, current.title_normalized, current.track_id
    FROM current
    ORDER BY current.artist_id, current.title_normalized, current.track_id
    ON CONFLICT DO NOTHING
    RETURNING track_id
)
SELECT (SELECT count(*) FROM removed) AS "removed!",
       (SELECT count(*) FROM written) AS "written!"
