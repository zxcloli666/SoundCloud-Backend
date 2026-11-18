WITH batch AS MATERIALIZED (
    SELECT track.id,
           track.sc_track_id,
           track.needs_duration_resolve = false AS ready_home,
           track.index_state = 'indexed' AS ready_wave,
           track.sharing = 'public'
               AND track.superseded_by IS NULL
               AND track.storage_state = 'ok'
               AND (track.needs_duration_resolve = false OR track.index_state = 'indexed') AS playable
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
    SELECT credit.artist_id,
           batch.id AS track_id,
           batch.sc_track_id,
           batch.ready_home,
           batch.ready_wave
    FROM batch
    JOIN track_artists AS credit ON credit.track_id = batch.id AND credit.role = 'primary'
    WHERE batch.playable
), stored AS MATERIALIZED (
    SELECT playable.artist_id,
           playable.track_id,
           playable.sc_track_id,
           playable.ready_home,
           playable.ready_wave
    FROM bounds
    JOIN artist_playable_tracks AS playable ON playable.track_id > $1 AND playable.track_id <= bounds.upper
)
SELECT bounds.last AS "last?",
       bounds.scanned AS "scanned!",
       ARRAY(
           SELECT DISTINCT coalesce(expected.track_id, stored.track_id)
           FROM expected
           FULL JOIN stored ON stored.artist_id = expected.artist_id AND stored.track_id = expected.track_id
           WHERE expected.track_id IS NULL
              OR stored.track_id IS NULL
              OR (expected.sc_track_id, expected.ready_home, expected.ready_wave)
                 IS DISTINCT FROM
                 (stored.sc_track_id, stored.ready_home, stored.ready_wave)
           ORDER BY 1
       ) AS "mismatched!"
FROM bounds
