INSERT INTO taste_model_versions (
    version,
    input_object,
    collection,
    trained_at,
    dim,
    pooling,
    metrics,
    items_count,
    users_count
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
ON CONFLICT (version) DO UPDATE
SET pooling = EXCLUDED.pooling,
    metrics = EXCLUDED.metrics,
    users_count = EXCLUDED.users_count
WHERE taste_model_versions.input_object = EXCLUDED.input_object
  AND taste_model_versions.applied_at IS NULL
