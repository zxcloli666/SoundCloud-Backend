WITH strict AS MATERIALIZED (
    SELECT id, sc_track_id, title, title_normalized, metadata_artist, uploader_username, duration_ms, play_count_sc
    FROM tracks
    WHERE to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, '')) @@ $1::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
    LIMIT 2000
),
loose AS MATERIALIZED (
    SELECT id, sc_track_id, title, title_normalized, metadata_artist, uploader_username, duration_ms, play_count_sc
    FROM tracks
    WHERE $2::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, '')) @@ $2::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
    LIMIT 2000
),
stemmed AS MATERIALIZED (
    SELECT id, sc_track_id, title, title_normalized, metadata_artist, uploader_username, duration_ms, play_count_sc
    FROM tracks
    WHERE $4::text IS NOT NULL
      AND (SELECT count(*) FROM strict) < 50
      AND to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, '')) @@ $4::text::tsquery
      AND sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
    LIMIT 2000
),
hits AS MATERIALIZED (
    SELECT DISTINCT ON (id) id, sc_track_id, tier, title, title_normalized, duration_ms, play_count_sc,
           coalesce(nullif(metadata_artist, ''), uploader_username, '') AS artist,
           search_latin(title_normalized || ' ' || coalesce(nullif(metadata_artist, ''), uploader_username, '')) AS doc
    FROM (SELECT strict.*, 0 AS tier FROM strict
          UNION ALL SELECT stemmed.*, 1 FROM stemmed
          UNION ALL SELECT loose.*, 1 FROM loose) found
    ORDER BY id, tier
),
scored AS (
    SELECT id, sc_track_id, title, tier, doc, duration_ms, artist,
           least(1, ln(1 + coalesce(play_count_sc, 0)) / 18)::float8 AS popularity,
           (least(2, similarity(search_latin($3), doc) + word_similarity(search_latin($3), doc)) / 2)::float8
             - CASE WHEN title_normalized ~ '\m(remix|sped up|slowed|nightcore|cover|live|8d|reverb|bass boosted|instrumental|karaoke|mashup|edit)\M' AND $3 !~ '\m(remix|sped up|slowed|nightcore|cover|live|8d|reverb|bass boosted|instrumental|karaoke|mashup|edit)\M' THEN 0.25 ELSE 0 END AS match_score
    FROM hits
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
