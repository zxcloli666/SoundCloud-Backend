UPDATE artist_attribution_revalidation
SET credits_removed = credits_removed + $1,
    tracks_reset    = tracks_reset + $2,
    updated_at      = now()
WHERE singleton
