WITH strict AS MATERIALIZED (
    SELECT id FROM tracks
    WHERE to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, '')) @@ $1::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
      AND ($4::text IS NULL OR uploader_sc_user_id = $4)
    LIMIT 2000
),
loose AS MATERIALIZED (
    SELECT id FROM tracks
    WHERE $2::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, '')) @@ $2::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
      AND ($4::text IS NULL OR uploader_sc_user_id = $4)
    LIMIT 2000
),
uploader AS MATERIALIZED (
    SELECT own.id
    FROM users u
    CROSS JOIN LATERAL (
        SELECT id FROM tracks
        WHERE uploader_sc_user_id = u.sc_user_id
          AND sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
        ORDER BY play_count_sc DESC NULLS LAST, id DESC
        LIMIT 100) own
    WHERE $4::text IS NULL AND u.username_normalized = $3
),
hits AS (
    SELECT id, min(tier) AS tier
    FROM (SELECT id, 0 AS tier FROM strict
          UNION ALL SELECT id, 0 FROM uploader
          UNION ALL SELECT id, 1 FROM loose) found
    GROUP BY id
),
scored AS (
    SELECT t.id, t.sc_track_id, hits.tier, d.doc, t.duration_ms,
           similarity(search_latin($3), d.doc) + word_similarity(search_latin($3), d.doc)
             + 0.3 * least(1, ln(1 + coalesce(t.play_count_sc, 0)) / 18)
             - CASE WHEN NOT $7 AND t.title_normalized ~ '\m(remix|sped up|slowed|nightcore|cover|live|8d|reverb|bass boosted|instrumental|karaoke|mashup|edit)\M' THEN 0.25 ELSE 0 END AS score
    FROM hits
    JOIN tracks t ON t.id = hits.id
    CROSS JOIN LATERAL (SELECT search_latin(t.title_normalized || ' ' || coalesce(nullif(t.metadata_artist, ''), t.uploader_username, '')) AS doc) d
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
