WITH strict AS MATERIALIZED (
    SELECT sc_track_id FROM lyrics_cache
    WHERE fts @@ $1::text::tsquery AND NOT (source = 'self_gen' AND plain_source IS NULL)
    LIMIT 300
),
loose AS MATERIALIZED (
    SELECT sc_track_id FROM lyrics_cache
    WHERE $2::text IS NOT NULL AND (SELECT count(*) FROM strict) < 20
      AND fts @@ $2::text::tsquery AND NOT (source = 'self_gen' AND plain_source IS NULL)
    LIMIT 300
),
hits AS (
    SELECT sc_track_id, min(tier) AS tier
    FROM (SELECT sc_track_id, 0 AS tier FROM strict UNION ALL SELECT sc_track_id, 1 FROM loose) found
    GROUP BY sc_track_id
),
lines AS (
    SELECT hits.sc_track_id, hits.tier, t.play_count_sc, btrim(line) AS line,
           word_similarity(search_latin($3), search_latin(line)) AS score
    FROM hits
    JOIN tracks t ON t.sc_track_id = hits.sc_track_id
     AND t.sharing = 'public' AND t.deleted_at IS NULL AND t.superseded_by IS NULL
    JOIN lyrics_cache lc ON lc.sc_track_id = hits.sc_track_id
    CROSS JOIN LATERAL regexp_split_to_table(
        coalesce(lc.plain_text, regexp_replace(coalesce(lc.synced_lrc, ''), '\[[0-9:.]+\]', '', 'g')), E'\n') AS line
    WHERE btrim(line) <> ''
),
best AS (
    SELECT DISTINCT ON (sc_track_id) sc_track_id, tier, play_count_sc, line, score
    FROM lines ORDER BY sc_track_id, score DESC
)
SELECT sc_track_id AS "sc_track_id!", left(line, 160) AS "matched_line!", score AS "score!"
FROM best
ORDER BY tier, score DESC, play_count_sc DESC NULLS LAST, sc_track_id DESC
LIMIT $4 OFFSET $5
