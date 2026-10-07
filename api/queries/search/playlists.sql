WITH strict AS MATERIALIZED (
    SELECT urn FROM playlists
    WHERE to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(owner_username, '')) @@ $1::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL
      AND ($4::text IS NULL OR owner_sc_user_id = $4)
    LIMIT 2000
),
loose AS MATERIALIZED (
    SELECT urn FROM playlists
    WHERE $2::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(owner_username, '')) @@ $2::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL
      AND ($4::text IS NULL OR owner_sc_user_id = $4)
    LIMIT 2000
),
hits AS (
    SELECT urn, min(tier) AS tier
    FROM (SELECT urn, 0 AS tier FROM strict UNION ALL SELECT urn, 1 FROM loose) found
    GROUP BY urn
),
scored AS (
    SELECT hits.urn, hits.tier,
           similarity(search_latin($3), d.doc) + word_similarity(search_latin($3), d.doc)
             + 0.3 * least(1, ln(1 + coalesce(p.likes_count_sc, 0)) / 18) AS score
    FROM hits
    JOIN playlists p ON p.urn = hits.urn
    CROSS JOIN LATERAL (SELECT search_latin(p.title_normalized || ' ' || coalesce(p.owner_username, '')) AS doc) d
)
SELECT p.urn,
       p.sc_playlist_id,
       p.title,
       p.title_normalized,
       p.description,
       p.genre,
       p.tags,
       p.artwork_url,
       p.permalink_url,
       p.owner_sc_user_id,
       p.owner_urn,
       p.owner_username,
       p.track_count,
       p.duration_ms,
       p.playlist_type,
       p.kind,
       p.sharing,
       p.sc_metadata,
       p.deleted_at,
       p.release_year,
       p.release_date,
       p.label_name,
       p.likes_count_sc,
       p.reposts_count_sc,
       p.sc_created_at,
       p.sc_last_modified,
       p.sc_synced_at,
       p.last_read_at,
       p.created_at,
       p.updated_at
FROM scored
JOIN playlists p ON p.urn = scored.urn
ORDER BY scored.tier, scored.score DESC, p.urn DESC
LIMIT $5 OFFSET $6
