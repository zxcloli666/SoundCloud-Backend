vh_db=scd_b1_verify_hash

say "verify hash: a typed 64-bit hash of each row's columns, many times cheaper per probe than the md5 of its jsonb text, the same answers"
fresh "$vh_db" scd_b1_tpl
sql -d "$vh_db" -f - > /dev/null <<'SQL'
SET session_replication_role = replica;
INSERT INTO public.tracks (id, sc_track_id, urn, title, title_normalized, description, genre, tags, artwork_url, permalink_url, waveform_url,
    duration_ms, sharing, uploader_sc_user_id, uploader_urn, uploader_username, uploader_avatar_url, audio_fingerprint,
    likes_count_sc, play_count_sc, reposts_count_sc, comments_count_sc, sc_created_at, sc_last_modified, sc_synced_at, indexed_at, enriched_at,
    sc_metadata, sc_desired, metadata_artist, isrc, release_date, work_key, recording_key)
SELECT md5('vh track ' || i)::uuid, (100000000 + i)::text, 'soundcloud:tracks:' || (100000000 + i), 'Track title number ' || i, 'track title number ' || i,
    CASE WHEN i % 3 <> 0 THEN repeat('Some description text with links and credits ', 1 + i % 12) END,
    'Electronic', ARRAY['house', 'deep house', 'tag ' || (i % 50), 'mix', 'set ' || (i % 7)],
    'https://i1.sndcdn.com/artworks-' || md5(i::text) || '-large.jpg', 'https://soundcloud.com/user' || (i % 9000) || '/track-title-number-' || i,
    'https://wave.sndcdn.com/' || md5((i * 7)::text) || '_m.png',
    180000 + i % 120000, 'public', (5000000 + i % 9000)::text, 'soundcloud:users:' || (5000000 + i % 9000), 'user' || (i % 9000),
    'https://i1.sndcdn.com/avatars-' || md5((i * 3)::text) || '-large.jpg', CASE WHEN i % 2 = 0 THEN repeat(md5(i::text), 32) END,
    i % 5000, i * 13 % 1000000, i % 300, i % 40, '2026-01-01'::timestamptz - (i % 2000) * interval '1 day', '2026-01-01'::timestamptz - (i % 900) * interval '1 hour',
    '2026-01-01'::timestamptz - (i % 50) * interval '1 minute', '2026-01-01'::timestamptz - (i % 30) * interval '1 day', '2026-01-01'::timestamptz - (i % 20) * interval '1 day',
    (SELECT jsonb_object_agg('field_' || k, CASE WHEN k % 3 = 0 THEN to_jsonb(k * i) WHEN k % 3 = 1 THEN to_jsonb('value text for field ' || k || ' of ' || i) ELSE to_jsonb(k % 2 = 0) END)
     FROM generate_series(1, 40) k),
    CASE WHEN i % 4 = 0 THEN jsonb_build_object('title', 'Track title number ' || i, 'sharing', 'public') ELSE '{}'::jsonb END,
    CASE WHEN i % 5 = 0 THEN 'Label ' || (i % 300) END, CASE WHEN i % 5 = 0 THEN 'USRC1' || lpad(i::text, 7, '0') END,
    CASE WHEN i % 5 = 0 THEN date '2020-01-01' + i % 1500 END, 'work:' || md5((i % 4000)::text), 'rec:' || md5(i::text)
FROM generate_series(1, 10000) i;
INSERT INTO public.artists (id, name, normalized_name, source, genius_artist_id, bio, tags)
SELECT md5('vh artist ' || i)::uuid, 'Artist ' || i, 'vh artist ' || i, 'sc', CASE WHEN i % 2 = 0 THEN 'g' || i END,
    CASE WHEN i % 3 = 0 THEN repeat('Artist biography text ', 1 + i % 20) END, ARRAY['tag ' || (i % 40), 'genre ' || (i % 9)]
FROM generate_series(1, 10000) i;
INSERT INTO public.lyrics_cache (sc_track_id, plain_text, synced_lrc, source, language, content_hash)
SELECT 'vh' || i, repeat('Lyric line number ' || i || ' with some words in it' || chr(10), 60),
    repeat('[00:' || lpad((i % 60)::text, 2, '0') || '.00] Lyric line with some words' || chr(10), 80), 'lrclib', 'en', decode(md5(i::text), 'hex')
FROM generate_series(1, 5000) i;
ANALYZE public.tracks;
ANALYZE public.artists;
ANALYZE public.lyrics_cache;
SQL
awk '/^CREATE OR REPLACE FUNCTION public.scd_verify_chunk\(/,/^\$f\$;/' "$root/api/migrations/0221_scd_verify.sql" \
  | sed 's/^CREATE OR REPLACE FUNCTION public\.scd_verify_chunk(/CREATE OR REPLACE FUNCTION public.scd_verify_chunk_jsonb_md5(/' \
  | sql -d "$vh_db" -f - > /dev/null

sql -At -d "$vh_db" -f - > "$tmp/vh.cost" <<'SQL'
CREATE FUNCTION pg_temp.vh_cost(rel text, OUT before_ms numeric, OUT after_ms numeric, OUT answer_ms numeric, OUT chunks integer)
LANGUAGE plpgsql AS $f$
DECLARE
    shown text[] := public.scd_verify_shown(rel);
    los jsonb[];
    his jsonb[];
    ops jsonb[];
    lo jsonb;
    hi jsonb;
    c record;
    i integer;
    r integer;
    t0 timestamptz;
    took float8;
    b float8;
    a float8;
    w float8;
BEGIN
    LOOP
        hi := public.scd_verify_next(rel, lo, 500);
        SELECT * INTO c FROM public.scd_verify_chunk(rel, lo, hi, shown, false);
        los := los || lo;
        his := his || hi;
        ops := ops || jsonb_build_object('scd_op', 'probe', 'hv', 2, 'n', c.n, 'h', c.h, 'attempt', 0, 'cols', to_jsonb(shown));
        EXIT WHEN hi IS NULL OR array_length(los, 1) = 10;
        lo := hi;
    END LOOP;
    chunks := array_length(los, 1);
    FOR r IN 1..3 LOOP
        t0 := clock_timestamp();
        FOR i IN 1..chunks LOOP
            PERFORM public.scd_verify_chunk_jsonb_md5(rel, los[i], his[i], shown);
        END LOOP;
        took := extract(epoch FROM clock_timestamp() - t0) * 1000 / chunks;
        b := least(coalesce(b, took), took);
        t0 := clock_timestamp();
        FOR i IN 1..chunks LOOP
            PERFORM public.scd_verify_chunk(rel, los[i], his[i], shown, false);
        END LOOP;
        took := extract(epoch FROM clock_timestamp() - t0) * 1000 / chunks;
        a := least(coalesce(a, took), took);
        t0 := clock_timestamp();
        FOR i IN 1..chunks LOOP
            PERFORM public.scd_verify_answer(gen_random_uuid(), rel, jsonb_build_object('lo', los[i], 'hi', his[i]), ops[i]);
        END LOOP;
        took := extract(epoch FROM clock_timestamp() - t0) * 1000 / chunks;
        w := least(coalesce(w, took), took);
    END LOOP;
    before_ms := round(b::numeric, 2);
    after_ms := round(a::numeric, 2);
    answer_ms := round(w::numeric, 2);
END
$f$;
SELECT v.rel || ' ' || c.before_ms || ' ' || c.after_ms || ' ' || c.answer_ms || ' ' || c.chunks
FROM (VALUES ('tracks'), ('artists'), ('lyrics_cache')) v (rel) CROSS JOIN LATERAL pg_temp.vh_cost(v.rel) c;
SQL
while read -r rel before after answer chunks; do
  echo "  cost $rel: $before ms per 500-row chunk before, $after ms after ($(awk -v b="$before" -v a="$after" 'BEGIN { printf "%.1f", b / a }')x), a whole api answer $answer ms ($chunks chunks, best of 3)"
done < "$tmp/vh.cost"
read -r _ before after _ _ < "$tmp/vh.cost"
awk -v b="$before" -v a="$after" 'BEGIN { exit !(a * 5 <= b) }' \
  && echo "ok a tracks chunk hashes at least 5x cheaper" || { echo "FAIL tracks chunk: $before ms before, $after ms after"; exit 1; }
want "an api answers no probe of equal data" "$(val "$vh_db" "SELECT count(*) FROM public.repl_checksums WHERE tbl IN ('tracks', 'artists', 'lyrics_cache')")" 0
want "an api ignores a probe without hv (an old core's), even one whose hash differs" \
  "$(val "$vh_db" "SELECT public.scd_verify_answer(gen_random_uuid(), 'tracks', '{\"lo\": null, \"hi\": null}', '{\"scd_op\": \"probe\", \"n\": 1, \"h\": \"x\", \"attempt\": 0, \"cols\": [\"id\"]}')::text || ' ' || count(*) FROM public.repl_checksums WHERE tbl = 'tracks'")" "true 0"

sql -At -d "$vh_db" -f - > "$tmp/vh.detect" <<'SQL'
SET session_replication_role = replica;
CREATE FUNCTION pg_temp.vh_detect() RETURNS SETOF text
LANGUAGE plpgsql AS $f$
DECLARE
    shown text[] := public.scd_verify_shown('tracks');
    lo jsonb := public.scd_verify_next('tracks', NULL, 1000);
    hi jsonb := public.scd_verify_next('tracks', lo, 500);
    base record;
    after record;
    target uuid;
    change text;
    line text;
    changes text[] := ARRAY[
        'UPDATE public.tracks SET sc_metadata = jsonb_set(sc_metadata, ''{field_7}'', ''"changed"'') WHERE id = $1',
        'UPDATE public.tracks SET sc_metadata = sc_metadata || ''{"field_2": 30.5}'' WHERE id = $1',
        'UPDATE public.tracks SET sc_desired = ''[]'' WHERE id = $1',
        'UPDATE public.tracks SET tags = ARRAY[tags] WHERE id = $1',
        'UPDATE public.tracks SET tags[2] = ''Deep house'' WHERE id = $1',
        'UPDATE public.tracks SET updated_at = updated_at + interval ''1 microsecond'' WHERE id = $1',
        'UPDATE public.tracks SET description = '''' WHERE id = $1',
        'UPDATE public.tracks SET enrich_confidence = 0 WHERE id = $1',
        'UPDATE public.tracks SET title = title_normalized, title_normalized = title WHERE id = $1',
        'UPDATE public.tracks SET hq_upgrade_pending = NOT hq_upgrade_pending WHERE id = $1',
        'UPDATE public.tracks SET genre = NULL WHERE id = $1',
        'UPDATE public.tracks SET album_position = 0 WHERE id = $1'
    ];
BEGIN
    SELECT * INTO base FROM public.scd_verify_chunk('tracks', lo, hi, shown, true);
    SELECT t.id INTO target FROM public.tracks t
    WHERE (t.id) > ((lo ->> 'id')::uuid) AND (t.id) <= ((hi ->> 'id')::uuid)
      AND t.description IS NULL AND t.enrich_confidence IS NULL AND t.sc_desired = '{}' AND t.album_position IS NULL
    ORDER BY t.id LIMIT 1;
    FOREACH change IN ARRAY changes LOOP
        BEGIN
            EXECUTE change USING target;
            SELECT * INTO after FROM public.scd_verify_chunk('tracks', lo, hi, shown, true);
            line := (base.h <> after.h)::text || ' ' ||
                (SELECT count(*) FROM jsonb_each_text(base.keys) b JOIN jsonb_each_text(after.keys) a ON a.key = b.key WHERE a.value <> b.value)
                || ' ' || (after.keys ? jsonb_build_object('id', target)::text)::text || ' ' || split_part(change, ' ', 5);
            RAISE EXCEPTION 'undo';
        EXCEPTION WHEN raise_exception THEN
            RETURN NEXT line;
        END;
    END LOOP;
    SELECT * INTO after FROM public.scd_verify_chunk('tracks', lo, hi, shown, true);
    RETURN NEXT 'back ' || (base.h = after.h)::text;
END
$f$;
SELECT * FROM pg_temp.vh_detect();
SQL
caught="$(grep -c '^true 1 true ' "$tmp/vh.detect" || true)"
[ "$caught" = 12 ] && grep -qx 'back true' "$tmp/vh.detect" \
  && echo "ok 12 single-value changes of one tracks row (nested jsonb, jsonb {} to [], array nesting and element, 1 us, NULL to '' and 0, swapped columns) change the chunk hash and exactly that key" \
  || { echo "FAIL detection:"; cat "$tmp/vh.detect"; exit 1; }

sql -d "$vh_db" -f - > /dev/null <<'SQL'
DO $f$
DECLARE
    v text;
    q text;
BEGIN
    FOR v IN SELECT * FROM public.scd_verify_tables() LOOP
        SELECT format('SELECT pg_catalog.hash_record_extended(ROW(%s), 1) FROM (SELECT %s) t', public.scd_verify_cols(v, public.scd_verify_shown(v)),
                      string_agg(format('NULL::%s AS %I', format_type(a.atttypid, a.atttypmod), a.attname), ', '))
        INTO q
        FROM pg_attribute a WHERE a.attrelid = ('public.' || quote_ident(v))::regclass AND a.attnum > 0 AND NOT a.attisdropped;
        EXECUTE q;
    END LOOP;
END
$f$;
SQL
got="$(val "$vh_db" "SELECT count(*) || ' tables, ' || sum(cardinality(public.scd_verify_shown(v))) || ' columns' FROM public.scd_verify_tables() v")"
echo "ok the hash expression of every verified table hashes a row of its column types, so no type lacks a hash ($got)"
sql -d postgres -c "DROP DATABASE $vh_db WITH (FORCE)" > /dev/null
