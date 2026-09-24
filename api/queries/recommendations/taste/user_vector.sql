SELECT m.version, m.collection, v.vec
FROM user_taste_vectors v
JOIN taste_model_versions m ON m.version = v.version
WHERE m.active
  AND v.sc_user_id = $1
