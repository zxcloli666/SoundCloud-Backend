INSERT INTO wanted_tracks (title, normalized_title, primary_artist_id, source, external_id,
                           work_key, recording_key, work_normalizer_version)
VALUES ($1, $2, $3, 'genius_crawl', $4, $5, $6, $7)
ON CONFLICT (source, external_id) WHERE external_id IS NOT NULL DO UPDATE
    SET primary_artist_id       = COALESCE(wanted_tracks.primary_artist_id,
                                           EXCLUDED.primary_artist_id),
        work_key                = EXCLUDED.work_key,
        recording_key           = EXCLUDED.recording_key,
        work_normalizer_version = EXCLUDED.work_normalizer_version,
        updated_at              = now()
RETURNING id
