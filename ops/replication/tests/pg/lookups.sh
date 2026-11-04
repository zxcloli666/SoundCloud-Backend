lk_db=scd_b1_lookups
lk_name="FROM public.artists t WHERE t.normalized_name = NEW.normalized_name"
lk_genius="FROM public.artists t WHERE t.genius_artist_id = NEW.genius_artist_id"
lk_mb="FROM public.artists t WHERE t.mb_artist_id = NEW.mb_artist_id"
lk_spotify="FROM public.artists t WHERE t.spotify_artist_id = NEW.spotify_artist_id"
lk_lyrics="FROM public.lyrics_lookup_state t WHERE t.sc_track_id = NEW.sc_track_id"
lk_member="FROM public.playlist_track_projection t WHERE (t.playlist_urn, t.sc_track_id) = (NEW.playlist_urn, NEW.sc_track_id)"

lk_apply() {
  sql -d "$lk_db" -f - > /dev/null 2> "$tmp/lk.$1" <<SQL || { cat "$tmp/lk.$1"; exit 1; }
LOAD 'auto_explain';
SET auto_explain.log_min_duration = 0;
SET auto_explain.log_nested_statements = on;
SET auto_explain.log_analyze = on;
SET auto_explain.log_buffers = on;
SET auto_explain.log_timing = off;
SET auto_explain.log_parameter_max_length = 0;
SET auto_explain.log_level = notice;
SET scd.mode = 'split';
SET scd.role = 'api';
SET scd.node = 'star';
SET session_replication_role = replica;
$2
SQL
  awk '
    function blocks(s,   n, parts, kv, i) { n = 0; sub(/^.*shared /, "", s); split(s, parts, " "); for (i in parts) { split(parts[i], kv, "="); if (kv[1] == "hit" || kv[1] == "read") n += kv[2] } return n }
    function flush() { if (q != "") print q "\t" node "\t" blocks(buf); q = ""; node = ""; buf = "" }
    /Query Text: / { flush(); q = substr($0, index($0, "Query Text: ") + 12); next }
    q != "" && node == "" && NF > 0 { node = $0; sub(/^ +/, "", node); next }
    q != "" && buf == "" && /Buffers: shared/ { buf = $0 }
    END { flush() }
  ' "$tmp/lk.$1" > "$tmp/lk.$1.tsv"
}

lk_count() { grep -cF "$2" "$tmp/lk.$1.tsv" || true; }

lk_probes() {
  local bad
  bad="$(grep -F "$2" "$tmp/lk.$1.tsv" | awk -F'\t' -v idx="$3" 'index($2, "Index Scan using " idx " ") != 1 && index($2, "Index Only Scan using " idx " ") != 1 || $3 > 8 { print $2 " blocks=" $3 }' | head -3 || true)"
  [ -z "$bad" ] || { echo "FAIL $1: a lookup on $3 is not an index probe: $bad"; exit 1; }
}

lk_want() {
  local got=""
  for p in "$lk_name" "$lk_genius" "$lk_mb" "$lk_spotify" "$lk_lyrics" "$lk_member"; do
    got="$got $(lk_count "$1" "$p")"
  done
  want "$1: lookups per key (artists name genius mb spotify, lyrics_lookup_state sc_track_id, playlist_track_projection member)" "${got# }" "$2"
}

say "zz_owner: a replica insert looks a unique key up only on a conflict or a new value, each lookup is an index probe"
fresh "$lk_db" scd_b1_tpl
sql -d "$lk_db" -f - > /dev/null <<'SQL'
SET session_replication_role = replica;
INSERT INTO public.artists (id, name, normalized_name, source, genius_artist_id, mb_artist_id, spotify_artist_id)
SELECT md5('artist ' || i)::uuid, 'Artist ' || i, 'artist ' || i, 'sc',
    CASE WHEN i % 2 = 0 THEN 'g' || i END, CASE WHEN i % 3 = 0 THEN 'm' || i END, CASE WHEN i % 5 = 0 THEN 's' || i END
FROM generate_series(1, 20000) i;
INSERT INTO public.lyrics_lookup_state (track_id, sc_track_id, priority, input_title, input_artist, input_duration_ms, created_at)
SELECT md5('track ' || i)::uuid, 'sc' || i, 3, 'T' || i, 'A' || i, 1000, '2026-01-01' FROM generate_series(1, 20000) i;
INSERT INTO public.playlist_track_projection (playlist_urn, "position", sc_track_id)
SELECT 'urn:p:' || (i / 100), i % 100, 'sc' || i FROM generate_series(0, 19999) i;
ANALYZE public.artists;
ANALYZE public.lyrics_lookup_state;
ANALYZE public.playlist_track_projection;
SQL

lk_apply new "
INSERT INTO public.artists (id, name, normalized_name, source, genius_artist_id)
SELECT md5('new ' || i)::uuid, 'New ' || i, 'new ' || i, 'sc', 'gn' || i FROM generate_series(1, 8) i;
INSERT INTO public.lyrics_lookup_state (track_id, sc_track_id, priority, input_title, input_artist, input_duration_ms, created_at)
SELECT md5('new track ' || i)::uuid, 'new' || i, 3, 'T', 'A', 1000, '2026-01-01' FROM generate_series(1, 8) i;
INSERT INTO public.playlist_track_projection (playlist_urn, \"position\", sc_track_id)
SELECT 'urn:p:new', i, 'new' || i FROM generate_series(1, 8) i;"
lk_want new "0 0 0 0 0 0"
want "new: the rows are in" "$(val "$lk_db" "SELECT (SELECT count(*) FROM public.artists WHERE name LIKE 'New %') || ' ' || (SELECT count(*) FROM public.lyrics_lookup_state WHERE sc_track_id LIKE 'new%') || ' ' || (SELECT count(*) FROM public.playlist_track_projection WHERE playlist_urn = 'urn:p:new')")" "8 8 8"

lk_apply same "
INSERT INTO public.artists (id, name, normalized_name, source, genius_artist_id, mb_artist_id, spotify_artist_id)
SELECT md5('artist ' || i)::uuid, 'Renamed ' || i, 'artist ' || i, 'sc',
    CASE WHEN i % 2 = 0 THEN 'g' || i END, CASE WHEN i % 3 = 0 THEN 'm' || i END, CASE WHEN i % 5 = 0 THEN 's' || i END
FROM generate_series(1, 30) i;
INSERT INTO public.lyrics_lookup_state (track_id, sc_track_id, priority, input_title, input_artist, input_duration_ms, created_at)
SELECT md5('track ' || i)::uuid, 'sc' || i, 1, 'T' || i, 'A' || i, 1000, '2026-01-01' FROM generate_series(1, 8) i;
INSERT INTO public.playlist_track_projection (playlist_urn, \"position\", sc_track_id)
SELECT 'urn:p:1', i, 'sc' || (100 + i) FROM generate_series(0, 7) i;"
lk_want same "0 0 0 0 0 0"
want "same: the stored rows take the image and nothing is purged" "$(val "$lk_db" "SELECT (SELECT count(*) FROM public.artists WHERE name LIKE 'Renamed %') || ' ' || (SELECT count(*) FROM public.artists) || ' ' || (SELECT count(*) FROM public.lyrics_lookup_state WHERE priority = 1) || ' ' || (SELECT count(*) FROM public.playlist_track_projection)")" "30 20008 8 20008"

lk_apply conflict "
INSERT INTO public.artists (id, name, normalized_name, source)
SELECT md5('taker ' || i)::uuid, 'Taker ' || i, 'artist ' || (100 + i), 'sc' FROM generate_series(1, 8) i;
INSERT INTO public.lyrics_lookup_state (track_id, sc_track_id, priority, input_title, input_artist, input_duration_ms, created_at)
SELECT md5('taker track ' || i)::uuid, 'sc' || (100 + i), 3, 'T', 'A', 1000, '2026-01-01' FROM generate_series(1, 8) i;
INSERT INTO public.playlist_track_projection (playlist_urn, \"position\", sc_track_id)
SELECT 'urn:p:2', 100 + i, 'sc' || (200 + i) FROM generate_series(0, 7) i;"
lk_want conflict "8 0 0 0 8 8"
lk_probes conflict "$lk_name" artists_normalized_name_uq
lk_probes conflict "$lk_lyrics" lyrics_lookup_state_sc_track_id_key
lk_probes conflict "$lk_member" playlist_track_projection_member_uq
want "conflict: the holders are purged and the incoming rows take their keys" "$(val "$lk_db" "SELECT
    (SELECT count(*) FROM public.artists WHERE id IN (SELECT md5('artist ' || (100 + i))::uuid FROM generate_series(1, 8) i)) || ' ' ||
    (SELECT count(*) FROM public.artists WHERE name LIKE 'Taker %' AND normalized_name LIKE 'artist 10%') || ' ' ||
    (SELECT count(*) FROM public.lyrics_lookup_state WHERE track_id IN (SELECT md5('track ' || (100 + i))::uuid FROM generate_series(1, 8) i)) || ' ' ||
    (SELECT count(*) FROM public.lyrics_lookup_state WHERE track_id IN (SELECT md5('taker track ' || i)::uuid FROM generate_series(1, 8) i)) || ' ' ||
    (SELECT count(*) FROM public.playlist_track_projection WHERE playlist_urn = 'urn:p:2' AND \"position\" < 8) || ' ' ||
    (SELECT count(*) FROM public.playlist_track_projection WHERE playlist_urn = 'urn:p:2' AND \"position\" >= 100)")" "0 8 0 8 0 8"

lk_apply moved "
INSERT INTO public.artists (id, name, normalized_name, source, genius_artist_id, mb_artist_id, spotify_artist_id)
SELECT md5('artist ' || i)::uuid, 'Artist ' || i, 'artist ' || (100 + i), 'sc',
    CASE WHEN i % 2 = 0 THEN 'g' || i END, CASE WHEN i % 3 = 0 THEN 'm' || i END, CASE WHEN i % 5 = 0 THEN 's' || i END
FROM generate_series(201, 208) i;
INSERT INTO public.lyrics_lookup_state (track_id, sc_track_id, priority, input_title, input_artist, input_duration_ms, created_at)
SELECT md5('track ' || i)::uuid, 'sc' || (100 + i), 3, 'T' || i, 'A' || i, 1000, '2026-01-01' FROM generate_series(201, 208) i;
INSERT INTO public.playlist_track_projection (playlist_urn, \"position\", sc_track_id)
SELECT 'urn:p:3', i, 'sc' || (350 + i) FROM generate_series(0, 7) i;"
lk_want moved "8 0 0 0 8 8"
lk_probes moved "$lk_name" artists_normalized_name_uq
lk_probes moved "$lk_lyrics" lyrics_lookup_state_sc_track_id_key
lk_probes moved "$lk_member" playlist_track_projection_member_uq
want "moved: a stored row that takes another row's key purges that row" "$(val "$lk_db" "SELECT
    (SELECT count(*) FROM public.artists WHERE id IN (SELECT md5('artist ' || (300 + i))::uuid FROM generate_series(1, 8) i)) || ' ' ||
    (SELECT count(*) FROM public.artists WHERE id IN (SELECT md5('artist ' || i)::uuid FROM generate_series(201, 208) i) AND normalized_name LIKE 'artist 30%') || ' ' ||
    (SELECT count(*) FROM public.lyrics_lookup_state WHERE track_id IN (SELECT md5('track ' || (300 + i))::uuid FROM generate_series(1, 8) i)) || ' ' ||
    (SELECT count(*) FROM public.lyrics_lookup_state WHERE track_id IN (SELECT md5('track ' || i)::uuid FROM generate_series(201, 208) i) AND sc_track_id LIKE 'sc30%') || ' ' ||
    (SELECT count(*) FROM public.playlist_track_projection WHERE playlist_urn = 'urn:p:3' AND \"position\" BETWEEN 50 AND 57) || ' ' ||
    (SELECT count(*) FROM public.playlist_track_projection WHERE playlist_urn = 'urn:p:3' AND \"position\" < 8 AND sc_track_id LIKE 'sc35%')")" "0 8 0 8 0 8"

lk_apply update "
UPDATE public.artists SET normalized_name = 'artist ' || (100 + substr(name, 8)::int) WHERE id IN (SELECT md5('artist ' || i)::uuid FROM generate_series(401, 408) i);
UPDATE public.lyrics_lookup_state SET sc_track_id = 'sc' || (100 + substr(input_title, 2)::int) WHERE track_id IN (SELECT md5('track ' || i)::uuid FROM generate_series(401, 408) i);"
lk_want update "8 0 0 0 8 0"
lk_probes update "$lk_name" artists_normalized_name_uq
lk_probes update "$lk_lyrics" lyrics_lookup_state_sc_track_id_key
want "update: an applied key change purges the holder as before" "$(val "$lk_db" "SELECT
    (SELECT count(*) FROM public.artists WHERE id IN (SELECT md5('artist ' || (500 + i))::uuid FROM generate_series(1, 8) i)) || ' ' ||
    (SELECT count(*) FROM public.lyrics_lookup_state WHERE track_id IN (SELECT md5('track ' || (500 + i))::uuid FROM generate_series(1, 8) i))")" "0 0"

sed -n "/Query Text: SELECT t.id $lk_name/,/Buffers/p" "$tmp/lk.conflict" | tail -6 | sed 's/^/  plan: /'
lk_most="$(cat "$tmp"/lk.*.tsv | grep -F -e "$lk_name" -e "$lk_lyrics" -e "$lk_member" | awk -F'\t' '$3 > m { m = $3 } END { print m + 0 }')"
echo "ok every lookup of the conflict, moved and update cases is an index probe of at most 8 blocks (largest $lk_most)"
sql -d postgres -c "DROP DATABASE $lk_db WITH (FORCE)" > /dev/null
