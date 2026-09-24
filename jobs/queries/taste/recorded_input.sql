SELECT version, applied_at
FROM taste_model_versions
WHERE input_object = $1
