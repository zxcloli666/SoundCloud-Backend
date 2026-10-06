CREATE OR REPLACE FUNCTION search_latin(value text) RETURNS text
    LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
AS $$
SELECT translate(
    replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(lower(value),
        'щ', 'shch'), 'ж', 'zh'), 'х', 'kh'), 'ц', 'ts'), 'ч', 'ch'), 'ш', 'sh'), 'ю', 'yu'), 'я', 'ya'), 'є', 'ye'), 'ї', 'yi'),
    'абвгдеёзийклмнопрстуфыэіґáàâäãåéèêëíìîïóòôöõøúùûüýÿñçšžčъь',
    'abvgdeeziyklmnoprstufyeigaaaaaaeeeeiiiioooooouuuuyyncszc')
$$;

CREATE MATERIALIZED VIEW IF NOT EXISTS search_terms AS
SELECT word, sum(ndoc)::integer AS ndoc, search_latin(word) AS latin
FROM (
    SELECT doc.lexeme AS word, count(*) AS ndoc
    FROM tracks
    CROSS JOIN LATERAL unnest(to_tsvector('simple', coalesce(title_normalized, '') || ' ' || coalesce(metadata_artist, '') || ' ' || coalesce(uploader_username, ''))) AS doc
    WHERE sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
    GROUP BY doc.lexeme
    UNION ALL
    SELECT doc.lexeme, count(*)
    FROM lyrics_cache
    CROSS JOIN LATERAL unnest(fts) AS doc
    WHERE fts IS NOT NULL AND NOT (source = 'self_gen' AND plain_source IS NULL)
    GROUP BY doc.lexeme
) counted
WHERE char_length(word) BETWEEN 2 AND 40
GROUP BY word
HAVING sum(ndoc) >= 2;

CREATE UNIQUE INDEX IF NOT EXISTS search_terms_word_uq ON search_terms (word);
CREATE INDEX IF NOT EXISTS search_terms_latin_idx ON search_terms (latin text_pattern_ops);
CREATE INDEX IF NOT EXISTS search_terms_latin_trgm ON search_terms USING gin (latin gin_trgm_ops);
