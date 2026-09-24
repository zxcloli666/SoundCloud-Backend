INSERT INTO recommendation_quality_models (
    version,
    feature_means,
    feature_scales,
    weights,
    intercept,
    examples,
    positives,
    train_accuracy
)
SELECT COALESCE(max(version), 0) + 1,
       $1::real[],
       $2::real[],
       $3::real[],
       $4::real,
       $5::integer,
       $6::integer,
       $7::real
FROM recommendation_quality_models
RETURNING version
