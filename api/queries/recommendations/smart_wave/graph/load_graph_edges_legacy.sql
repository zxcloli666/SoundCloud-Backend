SELECT a_id AS "src!", b_id AS "dst!", weight::real AS "weight!", 0::int2 AS "kind!"
FROM artist_coplay
WHERE a_id = ANY ($1)
  AND weight > 0
UNION ALL
SELECT b_id, a_id, weight::real, 0::int2
FROM artist_coplay
WHERE b_id = ANY ($1)
  AND weight > 0
UNION ALL
SELECT a_id, b_id, w, 1::int2
FROM artist_colike
WHERE a_id = ANY ($1)
  AND w > 0
UNION ALL
SELECT b_id, a_id, w, 1::int2
FROM artist_colike
WHERE b_id = ANY ($1)
  AND w > 0
