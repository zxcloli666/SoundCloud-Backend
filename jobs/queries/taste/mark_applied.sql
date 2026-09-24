UPDATE taste_model_versions
SET applied_at = now()
WHERE version = $1
