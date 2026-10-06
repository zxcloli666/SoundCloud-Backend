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
hits AS (
    SELECT id, min(tier) AS tier
    FROM (SELECT id, 0 AS tier FROM strict UNION ALL SELECT id, 1 FROM loose) found
    GROUP BY id
),
scored AS (
    SELECT hits.id, hits.tier,
           similarity(search_latin($3), d.doc) + word_similarity(search_latin($3), d.doc)
             + 0.3 * coalesce(al.popularity_score, 0) AS score
    FROM hits
    JOIN albums al ON al.id = hits.id
    LEFT JOIN artists a ON a.id = al.primary_artist_id AND a.merged_into IS NULL
    CROSS JOIN LATERAL (SELECT search_latin(al.normalized_title || ' ' || coalesce(a.normalized_name, '')) AS doc) d
    WHERE al.track_count > 0
)
SELECT al.id,
       al.title,
       al.type      AS kind,
       al.release_year,
       al.release_date,
       al.cover_url,
       al.confidence,
       al.track_count,
       al.total_duration_ms,
       al.popularity_score,
       al.is_star_artist,
       al.primary_artist_id,
       a.name       AS "primary_artist_name?",
       a.avatar_url AS "primary_artist_avatar?"
FROM scored
JOIN albums al ON al.id = scored.id
LEFT JOIN artists a ON a.id = al.primary_artist_id AND a.merged_into IS NULL
ORDER BY scored.tier, scored.score DESC, al.id DESC
LIMIT $4 OFFSET $5
