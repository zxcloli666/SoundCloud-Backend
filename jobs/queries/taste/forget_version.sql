DELETE FROM taste_model_versions
WHERE version = $1
  AND NOT active
