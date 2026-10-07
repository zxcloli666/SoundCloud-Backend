CREATE TEMP TABLE search_terms_delta AS
WITH counted AS (
    SELECT doc.lexeme COLLATE "C" AS word, count(*) AS ndoc
    FROM tracks
    CROSS JOIN LATERAL unnest(to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, ''))) AS doc
    WHERE sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
    GROUP BY 1
    UNION ALL
    SELECT doc.lexeme COLLATE "C", count(*)
    FROM lyrics_cache
    CROSS JOIN LATERAL unnest(fts) AS doc
    WHERE fts IS NOT NULL AND NOT (source = 'self_gen' AND plain_source IS NULL)
    GROUP BY 1
),
fresh AS (
    SELECT word COLLATE "default" AS word, sum(ndoc)::integer AS ndoc, search_latin(word COLLATE "default") AS latin
    FROM counted
    WHERE char_length(word) BETWEEN 2 AND 40
    GROUP BY counted.word
    HAVING sum(ndoc) >= 2
),
changed AS (
    SELECT coalesce(fresh.word, stored.word) AS word, fresh.ndoc, fresh.latin
    FROM fresh
    FULL JOIN search_terms AS stored ON stored.word = fresh.word
    WHERE fresh.word IS NULL
       OR stored.word IS NULL
       OR stored.ndoc <> fresh.ndoc
       OR stored.latin <> fresh.latin
)
SELECT (row_number() OVER (ORDER BY word COLLATE "C") - 1) / $1 AS chunk, word, ndoc, latin
FROM changed
