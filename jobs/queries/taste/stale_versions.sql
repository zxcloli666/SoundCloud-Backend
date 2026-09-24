SELECT version, collection
FROM taste_model_versions
WHERE NOT active
ORDER BY trained_at DESC, version DESC
OFFSET $1
