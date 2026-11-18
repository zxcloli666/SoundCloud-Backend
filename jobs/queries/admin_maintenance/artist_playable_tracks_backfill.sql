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
), written AS (
    INSERT INTO artist_playable_tracks AS stored (
        artist_id, track_id, sc_track_id, ready_home, ready_wave
    )
    SELECT credit.artist_id,
           batch.id,
           batch.sc_track_id,
           batch.ready_home,
           batch.ready_wave
    FROM batch
    JOIN track_artists AS credit ON credit.track_id = batch.id AND credit.role = 'primary'
    WHERE batch.playable
    ORDER BY credit.artist_id, batch.id
    ON CONFLICT (artist_id, track_id) DO UPDATE
    SET sc_track_id = EXCLUDED.sc_track_id,
        ready_home = EXCLUDED.ready_home,
        ready_wave = EXCLUDED.ready_wave
    WHERE (stored.sc_track_id, stored.ready_home, stored.ready_wave)
          IS DISTINCT FROM
          (EXCLUDED.sc_track_id, EXCLUDED.ready_home, EXCLUDED.ready_wave)
    RETURNING stored.track_id
)
SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS "last?",
       (SELECT count(*) FROM batch) AS "scanned!",
       (SELECT count(*) FROM written) AS "inserted!"
