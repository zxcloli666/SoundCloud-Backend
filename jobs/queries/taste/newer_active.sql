SELECT EXISTS (
    SELECT 1
    FROM taste_model_versions
    WHERE active
      AND version <> $1
      AND trained_at > $2
) AS "newer!"
