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
ranked AS MATERIALIZED (
    SELECT hits.sc_track_id, hits.tier,
           row_number() OVER (
               ORDER BY hits.tier,
                        ts_rank(lc.fts, CASE WHEN hits.tier = 0 THEN $1 ELSE $2 END::tsquery) DESC,
                        t.play_count_sc DESC NULLS LAST,
                        hits.sc_track_id DESC
           ) AS pos
    FROM hits
    JOIN tracks t ON t.sc_track_id = hits.sc_track_id
     AND t.sharing = 'public' AND t.deleted_at IS NULL AND t.superseded_by IS NULL
    JOIN lyrics_cache lc ON lc.sc_track_id = hits.sc_track_id
),
needle AS MATERIALIZED (
    SELECT search_latin($3) AS latin
),
head AS (
    SELECT ranked.sc_track_id, best.line, best.score
    FROM ranked
    CROSS JOIN LATERAL (
        SELECT btrim(split.line) AS line, word_similarity(needle.latin, search_latin(split.line)) AS score
        FROM lyrics_cache lc, needle,
             regexp_split_to_table(
                 coalesce(lc.plain_text, regexp_replace(coalesce(lc.synced_lrc, ''), '\[[0-9:.]+\]', '', 'g')), E'\n'
             ) WITH ORDINALITY AS split(line, n)
        WHERE lc.sc_track_id = ranked.sc_track_id AND split.n <= 150 AND btrim(split.line) <> ''
        ORDER BY score DESC, split.n
        LIMIT 1
    ) best
    WHERE ranked.pos <= 40 AND $5::bigint < 40
),
page AS MATERIALIZED (
    SELECT ranked.sc_track_id, ranked.tier, ranked.pos, head.line, head.score
    FROM ranked
    LEFT JOIN head ON head.sc_track_id = ranked.sc_track_id
    ORDER BY ranked.tier, head.score DESC NULLS LAST, ranked.pos
    LIMIT $4 OFFSET $5
)
SELECT page.sc_track_id AS "sc_track_id!",
       left(coalesce(page.line, tail.line), 160) AS "matched_line!",
       coalesce(page.score, tail.score) AS "score!"
FROM page
LEFT JOIN LATERAL (
    SELECT btrim(split.line) AS line, word_similarity(needle.latin, search_latin(split.line)) AS score
    FROM lyrics_cache lc, needle,
         regexp_split_to_table(
             coalesce(lc.plain_text, regexp_replace(coalesce(lc.synced_lrc, ''), '\[[0-9:.]+\]', '', 'g')), E'\n'
         ) WITH ORDINALITY AS split(line, n)
    WHERE page.line IS NULL AND lc.sc_track_id = page.sc_track_id AND split.n <= 150 AND btrim(split.line) <> ''
    ORDER BY score DESC, split.n
    LIMIT 1
) tail ON true
WHERE coalesce(page.line, tail.line) IS NOT NULL
ORDER BY page.tier, page.score DESC NULLS LAST, page.pos
