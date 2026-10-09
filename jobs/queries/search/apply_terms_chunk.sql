WITH batch AS (
    SELECT word, ndoc, latin FROM search_terms_delta WHERE chunk = $1
),
upserted AS (
    INSERT INTO search_terms (word, ndoc, latin)
    SELECT word, ndoc, latin FROM batch WHERE ndoc IS NOT NULL
    ON CONFLICT (word) DO UPDATE
    SET ndoc = excluded.ndoc, latin = excluded.latin
    WHERE (search_terms.ndoc, search_terms.latin) IS DISTINCT FROM (excluded.ndoc, excluded.latin)
    RETURNING 1
),
removed AS (
    DELETE FROM search_terms
    USING batch
    WHERE batch.ndoc IS NULL AND search_terms.word = batch.word
    RETURNING 1
)
SELECT (SELECT count(*) FROM upserted) AS upserted, (SELECT count(*) FROM removed) AS removed
