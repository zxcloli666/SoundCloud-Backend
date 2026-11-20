WITH strict AS MATERIALIZED (
    SELECT sc_track_id, ts_rank(fts, $1::text::tsquery) AS rank FROM lyrics_cache
    WHERE fts @@ $1::text::tsquery AND NOT (source = 'self_gen' AND plain_source IS NULL)
    LIMIT 300
),
loose AS MATERIALIZED (
    SELECT sc_track_id, ts_rank(fts, $2::text::tsquery) AS rank FROM lyrics_cache
    WHERE $2::text IS NOT NULL AND (SELECT count(*) FROM strict) < 20
      AND fts @@ $2::text::tsquery AND NOT (source = 'self_gen' AND plain_source IS NULL)
    LIMIT 300
),
hits AS MATERIALIZED (
    SELECT DISTINCT ON (found.sc_track_id) found.sc_track_id, found.tier, found.rank
    FROM (SELECT sc_track_id, 0 AS tier, rank FROM strict UNION ALL SELECT sc_track_id, 1, rank FROM loose) found
    ORDER BY found.sc_track_id, found.tier
),
boundary AS MATERIALIZED (
    SELECT hits.tier, hits.rank FROM hits
    ORDER BY hits.tier, hits.rank DESC
    OFFSET greatest(64, $4 + $5) + 16
    LIMIT 1
),
front AS MATERIALIZED (
    SELECT hits.sc_track_id, hits.tier, hits.rank, t.play_count_sc
    FROM hits
    JOIN tracks t ON t.sc_track_id = hits.sc_track_id
     AND t.sharing = 'public' AND t.deleted_at IS NULL AND t.superseded_by IS NULL
    WHERE NOT EXISTS (SELECT 1 FROM boundary)
       OR hits.tier < (SELECT boundary.tier FROM boundary)
       OR (hits.tier = (SELECT boundary.tier FROM boundary) AND hits.rank >= (SELECT boundary.rank FROM boundary))
),
covered AS MATERIALIZED (
    SELECT (SELECT count(*) FROM front) >= greatest(64, $4 + $5) OR NOT EXISTS (SELECT 1 FROM boundary) AS ok
),
ranked AS MATERIALIZED (
    SELECT picked.sc_track_id, picked.tier,
           row_number() OVER (ORDER BY picked.tier, picked.rank DESC, picked.play_count_sc DESC NULLS LAST, picked.sc_track_id DESC) AS pos
    FROM (
        SELECT front.sc_track_id, front.tier, front.rank, front.play_count_sc FROM front WHERE (SELECT ok FROM covered)
        UNION ALL
        SELECT hits.sc_track_id, hits.tier, hits.rank, t.play_count_sc
        FROM hits
        JOIN tracks t ON t.sc_track_id = hits.sc_track_id
         AND t.sharing = 'public' AND t.deleted_at IS NULL AND t.superseded_by IS NULL
        WHERE NOT (SELECT ok FROM covered)
    ) picked
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
        WHERE lc.sc_track_id = ranked.sc_track_id AND split.n <= 150 AND btrim(split.line) <> '' AND btrim(split.line) !~ '^\['
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
    WHERE page.line IS NULL AND lc.sc_track_id = page.sc_track_id AND split.n <= 150 AND btrim(split.line) <> '' AND btrim(split.line) !~ '^\['
    ORDER BY score DESC, split.n
    LIMIT 1
) tail ON true
WHERE coalesce(page.line, tail.line) IS NOT NULL
  AND coalesce(page.score, tail.score) >= 0.6
ORDER BY page.tier, page.score DESC NULLS LAST, page.pos
