WITH strict AS MATERIALIZED (
    SELECT id FROM artists
    WHERE to_tsvector('simple', coalesce(normalized_name, '')) @@ $1::text::tsquery
      AND merged_into IS NULL
    LIMIT 500
),
loose AS MATERIALIZED (
    SELECT id FROM artists
    WHERE $2::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(normalized_name, '')) @@ $2::text::tsquery
      AND merged_into IS NULL
    LIMIT 500
),
stemmed AS MATERIALIZED (
    SELECT id FROM artists
    WHERE $4::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(normalized_name, '')) @@ $4::text::tsquery
      AND merged_into IS NULL
    LIMIT 500
),
hits AS (
    SELECT id, min(tier) AS tier
    FROM (SELECT id, 0 AS tier FROM strict
          UNION ALL SELECT id, 1 FROM stemmed
          UNION ALL SELECT id, 1 FROM loose) found
    GROUP BY id
)
SELECT a.id::text AS "id!",
       a.name AS "name!",
       ''::text AS "artist!",
       least(1, a.popularity_score)::float8 AS "popularity!",
       (least(2, similarity(search_latin($3), search_latin(a.normalized_name)) + word_similarity(search_latin($3), search_latin(a.normalized_name))) / 2
         + CASE WHEN a.normalized_name = $3 THEN 0.5 ELSE 0 END)::float8 AS "match_score!",
       hits.tier AS "tier!"
FROM hits
JOIN artists a ON a.id = hits.id
WHERE a.track_count_primary > 0 OR a.track_count_featured > 0
ORDER BY hits.tier,
         (least(2, similarity(search_latin($3), search_latin(a.normalized_name)) + word_similarity(search_latin($3), search_latin(a.normalized_name))) / 2
           + CASE WHEN a.normalized_name = $3 THEN 0.5 ELSE 0 END
           + 0.3 * least(1, a.popularity_score)) DESC,
         a.id DESC
LIMIT $5
