SELECT version,
       feature_means,
       feature_scales,
       weights,
       intercept
FROM recommendation_quality_models
ORDER BY version DESC
LIMIT 1
