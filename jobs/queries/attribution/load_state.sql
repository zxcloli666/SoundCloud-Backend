SELECT walker_completed_at,
       identity_cursor_artist,
       identity_cursor_account,
       identity_completed_at
FROM artist_attribution_revalidation
WHERE singleton
