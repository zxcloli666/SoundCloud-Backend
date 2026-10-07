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
hits AS (
    SELECT id, min(tier) AS tier
    FROM (SELECT id, 0 AS tier FROM strict UNION ALL SELECT id, 1 FROM loose) found
    GROUP BY id
),
scored AS (
    SELECT hits.id, hits.tier,
           similarity(search_latin($3), search_latin(a.normalized_name)) + word_similarity(search_latin($3), search_latin(a.normalized_name))
             + 0.3 * least(1, ln(1 + a.monthly_listeners) / 18)
             + CASE WHEN a.normalized_name = $3 THEN 1.0 ELSE 0 END AS score
    FROM hits
    JOIN artists a ON a.id = hits.id
    WHERE a.track_count_primary > 0 OR a.track_count_featured > 0
)
SELECT a.id,
       a.name,
       a.country,
       a.avatar_url,
       a.confidence,
       a.track_count_primary,
       a.track_count_featured,
       a.album_count_denorm,
       a.monthly_listeners,
       a.trending_score,
       a.tags,
       a.is_star,
       a.star_aura_id,
       a.star_custom_hex
FROM scored
JOIN artists a ON a.id = scored.id
ORDER BY scored.tier, scored.score DESC, a.id DESC
LIMIT $4 OFFSET $5
