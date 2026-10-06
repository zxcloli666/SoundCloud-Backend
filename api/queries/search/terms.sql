WITH tokens AS (
    SELECT v.lexeme AS token, min(p) AS ord
    FROM unnest(to_tsvector('simple', $1)) AS v
    CROSS JOIN LATERAL unnest(v.positions) AS p
    GROUP BY v.lexeme
    ORDER BY min(p)
    LIMIT $2
),
last AS (SELECT max(ord) AS ord FROM tokens)
SELECT t.ord AS "ord!", t.token AS "lexeme!", alt.word AS "word?", alt.ndoc AS "ndoc?", alt.kind AS "kind?"
FROM tokens t
CROSS JOIN last
LEFT JOIN LATERAL (
    (SELECT word, ndoc, 'same' AS kind FROM search_terms
     WHERE latin = search_latin(t.token) ORDER BY ndoc DESC LIMIT 4)
    UNION ALL
    (SELECT word, ndoc, 'near' FROM search_terms
     WHERE latin % search_latin(t.token)
       AND NOT EXISTS (SELECT 1 FROM search_terms s WHERE s.latin = search_latin(t.token))
     ORDER BY similarity(latin, search_latin(t.token)) DESC, ndoc DESC LIMIT 3)
    UNION ALL
    (SELECT word, ndoc, 'prefix' FROM search_terms
     WHERE t.ord = last.ord AND char_length(t.token) >= 2
       AND latin LIKE replace(replace(replace(search_latin(t.token), '\', '\\'), '%', '\%'), '_', '\_') || '%'
       AND latin <> search_latin(t.token)
     ORDER BY ndoc DESC LIMIT 5)
) alt ON true
ORDER BY t.ord
