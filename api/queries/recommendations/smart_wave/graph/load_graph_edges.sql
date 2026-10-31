WITH wanted AS (SELECT DISTINCT id FROM unnest($1::uuid[]) AS u(id))
SELECT e.a_id AS "src!", e.b_id AS "dst!", e.weight::real AS "weight!", 0::int2 AS "kind!"
FROM wanted w
         CROSS JOIN LATERAL (SELECT a_id, b_id, weight
                             FROM artist_coplay
                             WHERE a_id = w.id
                               AND weight > 0
                             ORDER BY weight DESC
                             LIMIT $2) e
UNION ALL
SELECT e.b_id, e.a_id, e.weight::real, 0::int2
FROM wanted w
         CROSS JOIN LATERAL (SELECT a_id, b_id, weight
                             FROM artist_coplay
                             WHERE b_id = w.id
                               AND weight > 0
                             ORDER BY weight DESC
                             LIMIT $2) e
UNION ALL
SELECT e.a_id, e.b_id, e.w, 1::int2
FROM wanted w
         CROSS JOIN LATERAL (SELECT a_id, b_id, w
                             FROM artist_colike
                             WHERE a_id = w.id
                               AND w > 0
                             ORDER BY w DESC
                             LIMIT $2) e
UNION ALL
SELECT e.b_id, e.a_id, e.w, 1::int2
FROM wanted w
         CROSS JOIN LATERAL (SELECT a_id, b_id, w
                             FROM artist_colike
                             WHERE b_id = w.id
                               AND w > 0
                             ORDER BY w DESC
                             LIMIT $2) e
