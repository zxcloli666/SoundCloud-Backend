WITH locked AS MATERIALIZED (
    SELECT track.id,
           track.sc_track_id,
           track.needs_duration_resolve = false AS ready_home,
           track.index_state = 'indexed' AS ready_wave,
           track.sharing = 'public'
               AND track.superseded_by IS NULL
               AND track.storage_state = 'ok'
               AND (track.needs_duration_resolve = false OR track.index_state = 'indexed') AS playable
    FROM tracks AS track
    WHERE track.id = ANY ($1)
    ORDER BY track.id
    FOR SHARE
), current AS MATERIALIZED (
    SELECT credit.artist_id,
           locked.id AS track_id,
           locked.sc_track_id,
           locked.ready_home,
           locked.ready_wave
    FROM locked
    JOIN track_artists AS credit ON credit.track_id = locked.id AND credit.role = 'primary'
    WHERE locked.playable
), removed AS (
    DELETE FROM artist_playable_tracks AS stored
    WHERE stored.track_id = ANY ($1)
      AND NOT EXISTS (
          SELECT 1
          FROM current
          WHERE current.artist_id = stored.artist_id
            AND current.track_id = stored.track_id
      )
    RETURNING stored.track_id
), written AS (
    INSERT INTO artist_playable_tracks AS stored (
        artist_id, track_id, sc_track_id, ready_home, ready_wave
    )
    SELECT current.artist_id,
           current.track_id,
           current.sc_track_id,
           current.ready_home,
           current.ready_wave
    FROM current
    ORDER BY current.artist_id, current.track_id
    ON CONFLICT (artist_id, track_id) DO UPDATE
    SET sc_track_id = EXCLUDED.sc_track_id,
        ready_home = EXCLUDED.ready_home,
        ready_wave = EXCLUDED.ready_wave
    WHERE (stored.sc_track_id, stored.ready_home, stored.ready_wave)
          IS DISTINCT FROM
          (EXCLUDED.sc_track_id, EXCLUDED.ready_home, EXCLUDED.ready_wave)
    RETURNING stored.track_id
)
SELECT (SELECT count(*) FROM removed) AS "removed!",
       (SELECT count(*) FROM written) AS "written!"
