CREATE TABLE IF NOT EXISTS recommendation_quality_models (
    version bigint PRIMARY KEY CHECK (version > 0),
    feature_means real[] NOT NULL CHECK (cardinality(feature_means) = 10 AND array_position(feature_means, NULL) IS NULL),
    feature_scales real[] NOT NULL CHECK (cardinality(feature_scales) = 10 AND array_position(feature_scales, NULL) IS NULL),
    weights real[] NOT NULL CHECK (cardinality(weights) = 10 AND array_position(weights, NULL) IS NULL),
    intercept real NOT NULL,
    examples integer NOT NULL CHECK (examples > 0),
    positives integer NOT NULL CHECK (positives > 0 AND positives < examples),
    train_accuracy real NOT NULL CHECK (train_accuracy >= 0 AND train_accuracy <= 1),
    trained_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE tracks ADD COLUMN IF NOT EXISTS quality_model_version bigint;

CREATE INDEX IF NOT EXISTS tracks_quality_rescore_idx
    ON tracks ((COALESCE(quality_model_version, 0)), indexed_at DESC)
    WHERE quality_score IS NOT NULL
      AND indexed_at IS NOT NULL
      AND index_state = 'indexed'
      AND storage_state <> 'too_long'
      AND needs_duration_resolve = false;
