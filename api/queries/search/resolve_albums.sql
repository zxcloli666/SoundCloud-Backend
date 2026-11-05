WITH strict AS MATERIALIZED (
    SELECT id FROM albums
    WHERE to_tsvector('simple', coalesce(normalized_title, '')) @@ $1::text::tsquery
    LIMIT 500
),
loose AS MATERIALIZED (
    SELECT id FROM albums
    WHERE $2::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(normalized_title, '')) @@ $2::text::tsquery
    LIMIT 500
),
stemmed AS MATERIALIZED (
    SELECT id FROM albums
    WHERE $4::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(normalized_title, '')) @@ $4::text::tsquery
    LIMIT 500
),
hits AS (
    SELECT id, min(tier) AS tier
    FROM (SELECT id, 0 AS tier FROM strict
          UNION ALL SELECT id, 1 FROM stemmed
          UNION ALL SELECT id, 1 FROM loose) found
    GROUP BY id
),
scored AS (
    SELECT al.id, al.title, hits.tier,
           coalesce(a.name, '') AS artist,
           least(1, coalesce(al.popularity_score, 0))::float8 AS popularity,
           (least(2, similarity(search_latin($3), d.doc) + word_similarity(search_latin($3), d.doc)) / 2)::float8 AS match_score
    FROM hits
    JOIN albums al ON al.id = hits.id
    LEFT JOIN artists a ON a.id = al.primary_artist_id AND a.merged_into IS NULL
    CROSS JOIN LATERAL (SELECT search_latin(al.normalized_title || ' ' || coalesce(a.normalized_name, '')) AS doc) d
    WHERE al.track_count > 0
)
SELECT id::text AS "id!",
       title AS "name!",
       artist AS "artist!",
       popularity AS "popularity!",
       match_score AS "match_score!",
       tier AS "tier!"
FROM scored
ORDER BY tier, match_score + 0.3 * popularity DESC, id DESC
LIMIT $5
