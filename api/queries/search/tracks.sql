WITH strict AS MATERIALIZED (
    SELECT id, sc_track_id, title_normalized, metadata_artist, uploader_username, duration_ms, play_count_sc, variant
    FROM track_search
    WHERE to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, '')) @@ $1::text::tsquery
      AND ($4::text IS NULL OR uploader_sc_user_id = $4)
    LIMIT 2000
),
loose AS MATERIALIZED (
    SELECT id, sc_track_id, title_normalized, metadata_artist, uploader_username, duration_ms, play_count_sc, variant
    FROM track_search
    WHERE $2::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, '')) @@ $2::text::tsquery
      AND ($4::text IS NULL OR uploader_sc_user_id = $4)
    LIMIT 2000
),
uploader AS MATERIALIZED (
    SELECT own.id, own.sc_track_id, own.title_normalized, own.metadata_artist, own.uploader_username, own.duration_ms, own.play_count_sc, own.variant
    FROM users u
    CROSS JOIN LATERAL (
        SELECT id, sc_track_id, title_normalized, metadata_artist, uploader_username, duration_ms, play_count_sc, variant
        FROM track_search
        WHERE uploader_sc_user_id = u.sc_user_id
        ORDER BY play_count_sc DESC NULLS LAST, id DESC
        LIMIT 100) own
    WHERE $4::text IS NULL AND u.username_normalized = $3
),
hits AS MATERIALIZED (
    SELECT DISTINCT ON (id) id, sc_track_id, tier, variant, duration_ms, play_count_sc,
           search_latin(title_normalized || ' ' || coalesce(nullif(metadata_artist, ''), uploader_username, '')) AS doc
    FROM (SELECT strict.*, 0 AS tier FROM strict
          UNION ALL SELECT uploader.*, 0 FROM uploader
          UNION ALL SELECT loose.*, 1 FROM loose) found
    ORDER BY id, tier
),
scored AS (
    SELECT id, sc_track_id, tier, doc, duration_ms,
           similarity(search_latin($3), doc) + word_similarity(search_latin($3), doc)
             + 0.3 * least(1, ln(1 + coalesce(play_count_sc, 0)) / 18)
             - CASE WHEN NOT $7 AND variant THEN 0.25 ELSE 0 END AS score
    FROM hits
),
ranked AS (
    SELECT id, sc_track_id, tier, score,
           row_number() OVER (PARTITION BY doc, duration_ms / 5000 ORDER BY tier, score DESC, id DESC) AS copy
    FROM scored
)
SELECT sc_track_id AS "sc_track_id!"
FROM ranked
WHERE copy = 1
ORDER BY tier, score DESC, id DESC
LIMIT $5 OFFSET $6
