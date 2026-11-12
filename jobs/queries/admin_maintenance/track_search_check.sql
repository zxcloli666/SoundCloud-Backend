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
), bounds AS MATERIALIZED (
    SELECT last.id AS last,
           CASE
               WHEN (SELECT count(*) FROM batch) < $2 THEN 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid
               ELSE last.id
           END AS upper,
           (SELECT count(*) FROM batch) AS scanned
    FROM (SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS id) AS last
), expected AS MATERIALIZED (
    SELECT batch.id,
           batch.sc_track_id,
           batch.uploader_sc_user_id,
           batch.title,
           batch.title_normalized,
           batch.metadata_artist,
           batch.uploader_username,
           batch.duration_ms,
           batch.play_count_sc,
           track_search_variant(batch.title_normalized) AS variant
    FROM batch
    WHERE batch.searchable
), stored AS MATERIALIZED (
    SELECT search.id,
           search.sc_track_id,
           search.uploader_sc_user_id,
           search.title,
           search.title_normalized,
           search.metadata_artist,
           search.uploader_username,
           search.duration_ms,
           search.play_count_sc,
           search.variant
    FROM bounds
    JOIN track_search AS search ON search.id > $1 AND search.id <= bounds.upper
)
SELECT bounds.last AS "last?",
       bounds.scanned AS "scanned!",
       ARRAY(
           SELECT coalesce(expected.id, stored.id)
           FROM expected
           FULL JOIN stored ON stored.id = expected.id
           WHERE expected.id IS NULL
              OR stored.id IS NULL
              OR (expected.sc_track_id, expected.uploader_sc_user_id, expected.title, expected.title_normalized,
                  expected.metadata_artist, expected.uploader_username, expected.duration_ms,
                  expected.play_count_sc, expected.variant)
                 IS DISTINCT FROM
                 (stored.sc_track_id, stored.uploader_sc_user_id, stored.title, stored.title_normalized,
                  stored.metadata_artist, stored.uploader_username, stored.duration_ms,
                  stored.play_count_sc, stored.variant)
           ORDER BY 1
       ) AS "mismatched!"
FROM bounds
