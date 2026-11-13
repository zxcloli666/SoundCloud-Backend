WITH page AS MATERIALIZED (
    SELECT cache.sc_track_id
    FROM lyrics_cache AS cache
    WHERE cache.sc_track_id > $1
    ORDER BY cache.sc_track_id
    LIMIT $2
), locked AS MATERIALIZED (
    SELECT cache.sc_track_id,
           cache.created_at,
           cache.plain_text,
           position('… Read More' || chr(160) IN left(cache.plain_text, 1500)) AS marker
    FROM lyrics_cache AS cache
    JOIN page ON page.sc_track_id = cache.sc_track_id
    WHERE cache.source = 'genius'
      AND COALESCE(cache.plain_source, cache.source) = 'genius'
      AND cache.created_at < TIMESTAMP '2026-09-25 00:00:00'
      AND position('… Read More' || chr(160) IN left(cache.plain_text, 1500)) > 0
    ORDER BY cache.sc_track_id
    FOR UPDATE OF cache
), stripped AS MATERIALIZED (
    SELECT locked.sc_track_id,
           locked.created_at,
           locked.plain_text,
           btrim(
               regexp_replace(
                   substr(locked.plain_text, locked.marker + char_length('… Read More' || chr(160))),
                   '^\[Текст песни[^\]\n]*\]\s*',
                   ''
               ),
               E' \t\n\r\f\x0B'
           ) AS text
    FROM locked
), kept AS MATERIALIZED (
    SELECT stripped.sc_track_id, stripped.created_at, stripped.plain_text, stripped.text
    FROM stripped
    WHERE char_length(stripped.text) >= 21
), ledger AS (
    INSERT INTO lyrics_repair (
        sc_track_id, kind, cache_created_at, status, outcome, old_plain_text, settled_at
    )
    SELECT kept.sc_track_id, 'description', kept.created_at, 'done', 'stripped', kept.plain_text, now()
    FROM kept
    ORDER BY kept.sc_track_id
    ON CONFLICT (sc_track_id) DO NOTHING
    RETURNING sc_track_id
), updated AS (
    UPDATE lyrics_cache AS cache
    SET plain_text = kept.text,
        embedded_at = NULL,
        embedding_state = NULL,
        content_generation = cache.content_generation + 1,
        updated_at = now()
    FROM kept
    JOIN ledger ON ledger.sc_track_id = kept.sc_track_id
    WHERE cache.sc_track_id = kept.sc_track_id
    RETURNING cache.sc_track_id
)
SELECT (SELECT page.sc_track_id FROM page ORDER BY page.sc_track_id DESC LIMIT 1) AS "last?",
       (SELECT count(*) FROM page) AS "scanned!",
       (SELECT count(*) FROM updated) AS "stripped!"
