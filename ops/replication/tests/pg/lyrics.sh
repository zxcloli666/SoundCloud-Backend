open_sql="PREPARE open_req(text, bigint, text, varchar, bytea, varchar) AS $(cat "$root/jobs/queries/lyrics/open_embedding_request.sql")"
elig_sql="PREPARE elig(text[]) AS $(cat "$root/api/queries/recommendations/service/qdrant_io/lyrics_vector_eligibility.sql")"
ly_ids="ARRAY['ly1', 'ly2', 'ly3', 'ly4']"
elig() { sql -At -d "scd_b1_$1" -c "$elig_sql" -c "EXECUTE elig($ly_ids)" | paste -sd, -; }
joined() {
  only "$1" "SELECT coalesce(string_agg(cache.sc_track_id || '|' || wire.request_message_id, ',' ORDER BY cache.sc_track_id), '')
    FROM public.lyrics_cache cache
    JOIN public.lyrics_embedding_wire_state wire ON wire.sc_track_id = cache.sc_track_id
    WHERE cache.sc_track_id = ANY($ly_ids) AND cache.embedded_at IS NOT NULL AND cache.embedding_state = 'done'
      AND wire.request_message_id IS NOT NULL AND wire.lyrics_created_at = cache.created_at"
}
open_req() { sql -d "scd_b1_$1" -c "$open_sql" -c "EXECUTE open_req('$2', 1, 'text', 'en', '\\x00', '$3')" > /dev/null; }
finish() { sql -d "scd_b1_$1" -c "UPDATE public.lyrics_cache SET embedded_at = now(), embedding_state = 'done' WHERE sc_track_id = '$2'" > /dev/null; }
queue() {
  sql -d "scd_b1_$1" -c "UPDATE public.lyrics_embedding_wire_state SET status = 'quarantined', quarantine_reason = 'lyrics_changed' WHERE sc_track_id = '$2'" \
    -c "UPDATE public.lyrics_cache SET embedded_at = NULL, embedding_state = 'queued' WHERE sc_track_id = '$2'" > /dev/null
}

say "lyrics eligibility: api nodes answer from lyrics_cache, with the wire state table empty, and the answer is core's join"
for t in ly1 ly2 ly3 ly4; do
  sql -d scd_b1_core -c "INSERT INTO public.lyrics_cache (sc_track_id, source, plain_text, embedding_state) VALUES ('$t', 'lrclib', 'words', 'queued')" > /dev/null
done
open_req core ly1 lyr:ly1:1
open_req core ly2 lyr:ly2:1
open_req core ly3 lyr:ly3:1
finish core ly1
finish core ly3
sql -d scd_b1_core -c "UPDATE public.lyrics_cache SET embedded_at = now(), embedding_state = 'done' WHERE sc_track_id = 'ly4'" > /dev/null
calm
want "core: the cache answer equals the wire join" "$(elig core)" "$(joined core)"
want "core answers ly1 and ly3 only: ly2 is pending, ly4 never had a request" "$(elig core)" "ly1|lyr:ly1:1,ly3|lyr:ly3:1"
want "main has no wire rows" "$(only main "SELECT count(*) FROM public.lyrics_embedding_wire_state")" 0
want "star has no wire rows" "$(only star "SELECT count(*) FROM public.lyrics_embedding_wire_state")" 0
want "main answers as core" "$(elig main)" "$(elig core)"
want "star answers as core" "$(elig star)" "$(elig core)"

say "lyrics eligibility: a lyrics rewrite on core closes it on the api nodes, a reopen opens it with the new request id"
sql -d scd_b1_core -c "UPDATE public.lyrics_cache SET created_at = now() + interval '1 second' WHERE sc_track_id = 'ly3'" > /dev/null
calm
want "core drops ly3, its wire row names the old lyrics" "$(elig core)" "ly1|lyr:ly1:1"
want "main drops it" "$(elig main)" "ly1|lyr:ly1:1"
want "star drops it" "$(elig star)" "ly1|lyr:ly1:1"
queue core ly3
open_req core ly3 lyr:ly3:2
finish core ly3
calm
want "core is back with the new request id" "$(elig core)" "ly1|lyr:ly1:1,ly3|lyr:ly3:2"
want "main follows core" "$(elig main)" "$(elig core)"
want "star follows core" "$(elig star)" "$(elig core)"
want "core equals the wire join" "$(elig core)" "$(joined core)"

say "lyrics eligibility: a cut-off api node keeps its last answer and catches up"
pause main main_from_core
queue core ly1
open_req core ly1 lyr:ly1:2
wait_for scd_b1_core "SELECT embedding_request_id = 'lyr:ly1:2' FROM public.lyrics_cache WHERE sc_track_id = 'ly1'" "core opened the second request"
want "main still serves the old answer while cut off" "$(elig main)" "ly1|lyr:ly1:1,ly3|lyr:ly3:2"
resume main main_from_core
finish core ly1
wait_for scd_b1_main "SELECT embedding_request_id = 'lyr:ly1:2' FROM public.lyrics_cache WHERE sc_track_id = 'ly1'" "main receives the new request id"
calm
want "main equals core after the catch-up" "$(elig main)" "$(elig core)"

say "lyrics eligibility: legacy mode answers as the wire join"
fresh scd_b1_legacy_main scd_b1_tpl
for t in ly1 ly2 ly3 ly4; do
  sql -d scd_b1_legacy_main -c "INSERT INTO public.lyrics_cache (sc_track_id, source, plain_text, embedding_state) VALUES ('$t', 'lrclib', 'words', 'queued')" > /dev/null
done
open_req legacy_main ly1 lyr:ly1:1
open_req legacy_main ly2 lyr:ly2:1
open_req legacy_main ly3 lyr:ly3:1
finish legacy_main ly1
finish legacy_main ly3
sql -d scd_b1_legacy_main -c "UPDATE public.lyrics_cache SET created_at = now() + interval '1 second' WHERE sc_track_id = 'ly3'" > /dev/null
want "legacy: ly1 only, ly3 is closed by its rewrite" "$(elig legacy_main)" "ly1|lyr:ly1:1"
want "legacy: the join gives the same" "$(joined legacy_main)" "ly1|lyr:ly1:1"
queue legacy_main ly3
open_req legacy_main ly3 lyr:ly3:2
finish legacy_main ly3
want "legacy: a reopen gives the join's answer" "$(elig legacy_main)" "$(joined legacy_main)"
sql -d postgres -c "DROP DATABASE scd_b1_legacy_main WITH (FORCE)" > /dev/null
