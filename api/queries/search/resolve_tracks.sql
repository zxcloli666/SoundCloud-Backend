WITH strict AS MATERIALIZED (
    SELECT id FROM tracks
    WHERE to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, '')) @@ $1::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
    LIMIT 2000
),
loose AS MATERIALIZED (
    SELECT id FROM tracks
    WHERE $2::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, '')) @@ $2::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
    LIMIT 2000
),
stemmed AS MATERIALIZED (
    SELECT id FROM tracks
    WHERE $4::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, '')) @@ $4::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
    LIMIT 2000
),
hits AS (
    SELECT id, min(tier) AS tier
    FROM (SELECT id, 0 AS tier FROM strict
          UNION ALL SELECT id, 1 FROM stemmed
          UNION ALL SELECT id, 1 FROM loose) found
    GROUP BY id
),
scored AS (
    SELECT t.id, t.sc_track_id, t.title, hits.tier, d.doc, t.duration_ms,
           coalesce(nullif(t.metadata_artist, ''), t.uploader_username, '') AS artist,
           least(1, ln(1 + coalesce(t.play_count_sc, 0)) / 18)::float8 AS popularity,
           (least(2, similarity(search_latin($3), d.doc) + word_similarity(search_latin($3), d.doc)) / 2)::float8
             - CASE WHEN t.title_normalized ~ '\m(remix|sped up|slowed|nightcore|cover|live|8d|reverb|bass boosted|instrumental|karaoke|mashup|edit)\M' AND $3 !~ '\m(remix|sped up|slowed|nightcore|cover|live|8d|reverb|bass boosted|instrumental|karaoke|mashup|edit)\M' THEN 0.25 ELSE 0 END AS match_score
    FROM hits
    JOIN tracks t ON t.id = hits.id
    CROSS JOIN LATERAL (SELECT search_latin(t.title_normalized || ' ' || coalesce(nullif(t.metadata_artist, ''), t.uploader_username, '')) AS doc) d
),
ranked AS (
    SELECT *, row_number() OVER (PARTITION BY doc, duration_ms / 5000 ORDER BY tier, match_score DESC, id DESC) AS copy
    FROM scored
)
SELECT ('soundcloud:tracks:' || sc_track_id) AS "id!",
       title AS "name!",
       artist AS "artist!",
       popularity AS "popularity!",
       match_score AS "match_score!",
       tier AS "tier!"
FROM ranked
WHERE copy = 1
ORDER BY tier, match_score + 0.3 * popularity DESC, id DESC
LIMIT $5
