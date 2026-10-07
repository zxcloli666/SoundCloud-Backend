WITH strict AS MATERIALIZED (
    SELECT sc_user_id FROM users
    WHERE to_tsvector('simple', coalesce(username_normalized, '') || ' ' || coalesce(full_name, '')) @@ $1::text::tsquery
    LIMIT 2000
),
loose AS MATERIALIZED (
    SELECT sc_user_id FROM users
    WHERE $2::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(username_normalized, '') || ' ' || coalesce(full_name, '')) @@ $2::text::tsquery
    LIMIT 2000
),
hits AS (
    SELECT sc_user_id, min(tier) AS tier
    FROM (SELECT sc_user_id, 0 AS tier FROM strict UNION ALL SELECT sc_user_id, 1 FROM loose) found
    GROUP BY sc_user_id
),
scored AS (
    SELECT hits.sc_user_id, hits.tier,
           similarity(search_latin($3), d.doc) + word_similarity(search_latin($3), d.doc)
             + 0.3 * least(1, ln(1 + coalesce(u.followers_count, 0)) / 18)
             + CASE WHEN u.username_normalized = $3 THEN 1.0 ELSE 0 END AS score
    FROM hits
    JOIN users u ON u.sc_user_id = hits.sc_user_id
    CROSS JOIN LATERAL (SELECT search_latin(u.username_normalized || ' ' || coalesce(u.full_name, '')) AS doc) d
)
SELECT u.sc_user_id,
       u.urn,
       u.username,
       u.username_normalized,
       u.full_name,
       u.first_name,
       u.last_name,
       u.permalink,
       u.permalink_url,
       u.avatar_url,
       u.country,
       u.city,
       u.description,
       u.verified,
       u.followers_count,
       u.followings_count,
       u.tracks_count,
       u.playlists_count,
       u.reposts_count,
       u.comments_count,
       u.kind,
       u.sc_created_at,
       u.sc_last_modified,
       u.sc_synced_at,
       u.last_read_at,
       u.created_at,
       u.updated_at
FROM scored
JOIN users u ON u.sc_user_id = scored.sc_user_id
ORDER BY scored.tier, scored.score DESC, u.sc_user_id DESC
LIMIT $4 OFFSET $5
