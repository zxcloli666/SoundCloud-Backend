WITH strict AS MATERIALIZED (
    SELECT urn FROM playlists
    WHERE to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(owner_username, '')) @@ $1::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL
    LIMIT 2000
),
loose AS MATERIALIZED (
    SELECT urn FROM playlists
    WHERE $2::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(owner_username, '')) @@ $2::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL
    LIMIT 2000
),
stemmed AS MATERIALIZED (
    SELECT urn FROM playlists
    WHERE $4::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(owner_username, '')) @@ $4::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL
    LIMIT 2000
),
hits AS (
    SELECT urn, min(tier) AS tier
    FROM (SELECT urn, 0 AS tier FROM strict
          UNION ALL SELECT urn, 1 FROM stemmed
          UNION ALL SELECT urn, 1 FROM loose) found
    GROUP BY urn
),
scored AS (
    SELECT p.urn, p.title, hits.tier,
           coalesce(p.owner_username, '') AS artist,
           least(1, ln(1 + coalesce(p.likes_count_sc, 0)) / 18)::float8 AS popularity,
           (least(2, similarity(search_latin($3), d.doc) + word_similarity(search_latin($3), d.doc)) / 2)::float8 AS match_score
    FROM hits
    JOIN playlists p ON p.urn = hits.urn
    CROSS JOIN LATERAL (SELECT search_latin(p.title_normalized || ' ' || coalesce(p.owner_username, '')) AS doc) d
)
SELECT urn AS "id!",
       title AS "name!",
       artist AS "artist!",
       popularity AS "popularity!",
       match_score AS "match_score!",
       tier AS "tier!"
FROM scored
ORDER BY tier, match_score + 0.3 * popularity DESC, urn DESC
LIMIT $5
