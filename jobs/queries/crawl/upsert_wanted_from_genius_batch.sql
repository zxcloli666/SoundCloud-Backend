WITH input AS (
    SELECT row.title, row.normalized_title, row.primary_artist_id, row.external_id,
           row.work_key, row.recording_key
    FROM UNNEST($1::text[], $2::text[], $3::uuid[], $4::text[], $5::text[], $6::text[])
        AS row(title, normalized_title, primary_artist_id, external_id, work_key, recording_key)
), current AS (
    SELECT wanted.id,
           wanted.external_id,
           (wanted.primary_artist_id, wanted.work_key, wanted.recording_key,
            wanted.work_normalizer_version)
               IS NOT DISTINCT FROM (COALESCE(wanted.primary_artist_id, input.primary_artist_id),
                                     input.work_key, input.recording_key, $7::smallint) AS unchanged
    FROM input
             JOIN wanted_tracks AS wanted
                  ON wanted.source = 'genius_crawl' AND wanted.external_id = input.external_id
), upserted AS (
    INSERT INTO wanted_tracks (title, normalized_title, primary_artist_id, source, external_id,
                               work_key, recording_key, work_normalizer_version)
    SELECT input.title, input.normalized_title, input.primary_artist_id, 'genius_crawl',
           input.external_id, input.work_key, input.recording_key, $7::smallint
    FROM input
    WHERE NOT EXISTS (
        SELECT 1 FROM current WHERE current.external_id = input.external_id AND current.unchanged
    )
    ON CONFLICT (source, external_id) WHERE external_id IS NOT NULL DO UPDATE
        SET primary_artist_id       = COALESCE(wanted_tracks.primary_artist_id,
                                               EXCLUDED.primary_artist_id),
            work_key                = EXCLUDED.work_key,
            recording_key           = EXCLUDED.recording_key,
            work_normalizer_version = EXCLUDED.work_normalizer_version,
            updated_at              = now()
        WHERE (wanted_tracks.primary_artist_id, wanted_tracks.work_key,
               wanted_tracks.recording_key, wanted_tracks.work_normalizer_version)
            IS DISTINCT FROM (COALESCE(wanted_tracks.primary_artist_id, EXCLUDED.primary_artist_id),
                              EXCLUDED.work_key, EXCLUDED.recording_key,
                              EXCLUDED.work_normalizer_version)
    RETURNING id, external_id
)
SELECT upserted.id AS "id!", upserted.external_id AS "external_id!" FROM upserted
UNION ALL
SELECT wanted.id, wanted.external_id
FROM wanted_tracks AS wanted
WHERE wanted.source = 'genius_crawl'
  AND wanted.external_id = ANY ($4::text[])
  AND NOT EXISTS (SELECT 1 FROM upserted WHERE upserted.id = wanted.id)
