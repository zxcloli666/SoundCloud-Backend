UPDATE taste_model_versions
SET refreshed_through = $2
WHERE version = $1
