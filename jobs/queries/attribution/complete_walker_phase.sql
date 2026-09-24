UPDATE artist_attribution_revalidation
SET walker_completed_at = now(),
    updated_at          = now()
WHERE singleton
  AND walker_completed_at IS NULL
