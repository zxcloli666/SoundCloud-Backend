WITH batch AS MATERIALIZED (
    SELECT track.id,
           track.sc_track_id,
           track.uploader_sc_user_id,
           track.title,
           track.title_normalized,
           track.metadata_artist,
           track.uploader_username,
           track.duration_ms,
           track.play_count_sc,
           track.sharing = 'public' AND track.deleted_at IS NULL AND track.superseded_by IS NULL AS searchable
    FROM tracks AS track
    WHERE track.id > $1
    ORDER BY track.id
    LIMIT $2
), inserted AS (
    INSERT INTO track_search (
        id, sc_track_id, uploader_sc_user_id, title, title_normalized,
        metadata_artist, uploader_username, duration_ms, play_count_sc, variant
    )
    SELECT batch.id,
           batch.sc_track_id,
           batch.uploader_sc_user_id,
           batch.title,
           batch.title_normalized,
           batch.metadata_artist,
           batch.uploader_username,
           batch.duration_ms,
           batch.play_count_sc,
           track_search_variant(batch.title_normalized)
    FROM batch
    WHERE batch.searchable
    ORDER BY batch.id
    ON CONFLICT (id) DO NOTHING
    RETURNING id
)
SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS "last?",
       (SELECT count(*) FROM batch) AS "scanned!",
       (SELECT count(*) FROM inserted) AS "inserted!"
