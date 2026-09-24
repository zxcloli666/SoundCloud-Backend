UPDATE artist_attribution_revalidation
SET identity_completed_at = now(),
    updated_at            = now()
WHERE singleton
  AND identity_completed_at IS NULL
