UPDATE taste_model_versions
SET active = true,
    applied_at = now(),
    refreshed_through = $2
WHERE version = $1
