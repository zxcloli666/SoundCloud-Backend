UPDATE artist_attribution_revalidation
SET identity_cursor_artist  = $1,
    identity_cursor_account = $2,
    credits_removed         = credits_removed + $3,
    tracks_reset            = tracks_reset + $4,
    updated_at              = now()
WHERE singleton
