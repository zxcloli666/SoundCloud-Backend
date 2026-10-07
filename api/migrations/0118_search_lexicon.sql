CREATE OR REPLACE FUNCTION search_latin(value text) RETURNS text
    LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
AS $$
SELECT translate(
    replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(lower(value),
        'щ', 'shch'), 'ж', 'zh'), 'х', 'kh'), 'ц', 'ts'), 'ч', 'ch'), 'ш', 'sh'), 'ю', 'yu'), 'я', 'ya'), 'є', 'ye'), 'ї', 'yi'),
    'абвгдеёзийклмнопрстуфыэіґáàâäãåéèêëíìîïóòôöõøúùûüýÿñçšžčъь',
    'abvgdeeziyklmnoprstufyeigaaaaaaeeeeiiiioooooouuuuyyncszc')
$$;

CREATE TABLE IF NOT EXISTS search_terms (
    word text PRIMARY KEY,
    ndoc integer NOT NULL,
    latin text NOT NULL
) WITH (fillfactor = 85);

CREATE INDEX IF NOT EXISTS search_terms_latin_idx ON search_terms (latin text_pattern_ops);
CREATE INDEX IF NOT EXISTS search_terms_latin_trgm ON search_terms USING gin (latin gin_trgm_ops);
