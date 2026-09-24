UPDATE taste_model_versions
SET active = false
WHERE active
  AND version <> $1
