WITH locked AS MATERIALIZED (
    SELECT track.id,
           track.sc_synced_at
    FROM tracks AS track
    WHERE track.id = ANY ($1)
    ORDER BY track.id
    FOR SHARE
), removed AS (
    DELETE FROM track_sync_stamps AS stamp
    WHERE stamp.track_id = ANY ($1)
      AND NOT EXISTS (SELECT 1 FROM locked WHERE locked.id = stamp.track_id)
    RETURNING stamp.track_id
), written AS (
    INSERT INTO track_sync_stamps AS stamp (track_id, sc_synced_at)
    SELECT locked.id,
           locked.sc_synced_at
    FROM locked
    ORDER BY locked.id
    ON CONFLICT (track_id) DO UPDATE
    SET sc_synced_at = EXCLUDED.sc_synced_at
    WHERE stamp.sc_synced_at < EXCLUDED.sc_synced_at
    RETURNING stamp.track_id
)
SELECT (SELECT count(*) FROM removed) AS "removed!",
       (SELECT count(*) FROM written) AS "written!"
