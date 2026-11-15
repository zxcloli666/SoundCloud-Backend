WITH finalized AS (
    UPDATE tracks
    SET primary_artist_id  = $2,
        album_id           = $3,
        isrc               = $4,
        canonical_track_id = COALESCE($5, canonical_track_id),
        cover_of_artist_id = $6,
        release_date       = COALESCE($10, release_date, sc_created_at::date),
        release_year       = COALESCE(
                $11,
                EXTRACT(YEAR FROM $10::date) ::smallint,
                release_year,
                EXTRACT(YEAR FROM sc_created_at) ::smallint
                             ),
        enrich_state       = 'done',
        enrich_source      = $7,
        enrich_confidence  = $8,
        enriched_at        = now(),
        upload_kind        = $9,
        is_cover           = $12,
        genius_song_id     = COALESCE($13, genius_song_id),
        genius_url         = COALESCE($14, genius_url)
    WHERE id = $1
    RETURNING id
)
UPDATE track_enrich_queue
SET attempts = 0,
    locked_at = NULL,
    error = NULL
FROM finalized
WHERE track_id = finalized.id
