WITH locked AS MATERIALIZED (
    SELECT track.id,
           track.primary_artist_id,
           track.superseded_by IS NOT NULL AS superseded,
           track.work_key,
           track.recording_key,
           track.work_normalizer_version
    FROM tracks AS track
    WHERE track.id = ANY ($1)
    ORDER BY track.id
    FOR SHARE
), removed AS (
    DELETE FROM track_work_keys AS keys
    WHERE keys.track_id = ANY ($1)
      AND NOT EXISTS (SELECT 1 FROM locked WHERE locked.id = keys.track_id)
    RETURNING keys.track_id
), written AS (
    INSERT INTO track_work_keys AS stored (
        track_id, primary_artist_id, superseded, work_key, recording_key, work_normalizer_version
    )
    SELECT locked.id,
           locked.primary_artist_id,
           locked.superseded,
           locked.work_key,
           locked.recording_key,
           locked.work_normalizer_version
    FROM locked
    ORDER BY locked.id
    ON CONFLICT (track_id) DO UPDATE
    SET primary_artist_id = EXCLUDED.primary_artist_id,
        superseded = EXCLUDED.superseded
    WHERE (stored.primary_artist_id, stored.superseded)
          IS DISTINCT FROM (EXCLUDED.primary_artist_id, EXCLUDED.superseded)
    RETURNING stored.track_id
)
SELECT (SELECT count(*) FROM removed) AS "removed!",
       (SELECT count(*) FROM written) AS "written!"
