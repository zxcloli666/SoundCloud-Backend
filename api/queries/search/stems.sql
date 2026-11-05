SELECT t.token AS "token!", (ts_lexize('russian_stem', t.token))[1] AS "stem?"
FROM unnest($1::text[]) AS t(token)
