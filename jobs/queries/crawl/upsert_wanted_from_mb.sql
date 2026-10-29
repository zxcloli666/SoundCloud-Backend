WITH upserted AS (
    INSERT INTO wanted_tracks (title, normalized_title, primary_artist_id, isrc, duration_ms,
                               release_year, source, external_id, work_key, recording_key,
                               work_normalizer_version)
    VALUES ($1, $2, $3, $4, $5, $6, 'mb_crawl', $7, $8, $9, $10)
    ON CONFLICT (source, external_id) WHERE external_id IS NOT NULL DO UPDATE
        SET primary_artist_id       = COALESCE(wanted_tracks.primary_artist_id,
                                               EXCLUDED.primary_artist_id),
            isrc                    = COALESCE(wanted_tracks.isrc, EXCLUDED.isrc),
            duration_ms             = COALESCE(wanted_tracks.duration_ms, EXCLUDED.duration_ms),
            release_year            = COALESCE(wanted_tracks.release_year, EXCLUDED.release_year),
            work_key                = EXCLUDED.work_key,
            recording_key           = EXCLUDED.recording_key,
            work_normalizer_version = EXCLUDED.work_normalizer_version,
            updated_at              = now()
        WHERE (wanted_tracks.primary_artist_id, wanted_tracks.isrc, wanted_tracks.duration_ms,
               wanted_tracks.release_year, wanted_tracks.work_key, wanted_tracks.recording_key,
               wanted_tracks.work_normalizer_version)
            IS DISTINCT FROM (COALESCE(wanted_tracks.primary_artist_id, EXCLUDED.primary_artist_id),
                              COALESCE(wanted_tracks.isrc, EXCLUDED.isrc),
                              COALESCE(wanted_tracks.duration_ms, EXCLUDED.duration_ms),
                              COALESCE(wanted_tracks.release_year, EXCLUDED.release_year),
                              EXCLUDED.work_key, EXCLUDED.recording_key,
                              EXCLUDED.work_normalizer_version)
    RETURNING id
)
SELECT id AS "id!" FROM upserted
UNION ALL
SELECT id FROM wanted_tracks
WHERE source = 'mb_crawl' AND external_id = $7 AND NOT EXISTS (SELECT 1 FROM upserted)
LIMIT 1
