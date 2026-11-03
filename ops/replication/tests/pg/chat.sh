say "chat: the chat schema stays node-local"
for n in $nodes; do
  [ "$(val "scd_b1_$n" "SELECT count(*) FROM pg_catalog.pg_tables WHERE schemaname = 'chat'")" -gt 0 ] \
    || { echo "FAIL the chat schema is missing on $n"; exit 1; }
  [ "$(val "scd_b1_$n" "SELECT count(*) FROM pg_catalog.pg_publication_tables WHERE schemaname = 'chat'")" = 0 ] \
    || { echo "FAIL a chat table is published on $n"; exit 1; }
  [ "$(val "scd_b1_$n" "SELECT count(*) FROM pg_catalog.pg_publication WHERE puballtables")" = 0 ] \
    || { echo "FAIL a FOR ALL TABLES publication exists on $n"; exit 1; }
  [ "$(val "scd_b1_$n" "SELECT count(*) FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid JOIN pg_catalog.pg_namespace s ON s.oid = c.relnamespace WHERE s.nspname = 'chat' AND NOT t.tgisinternal")" = 0 ] \
    || { echo "FAIL a replication trigger sits on a chat table on $n"; exit 1; }
done
echo "ok chat tables exist on every node, in no publication and without triggers"
sql -d scd_b1_main -c "INSERT INTO chat.meta (id, incarnation) VALUES (1, '00000000-0000-0000-0000-0000000c4a00') ON CONFLICT (id) DO NOTHING" \
  -c "INSERT INTO chat.users (user_id) VALUES ('c4a')"
settle ec01 ec02 ec03
[ "$(val scd_b1_core "SELECT count(*) FROM chat.meta") $(val scd_b1_core "SELECT count(*) FROM chat.users")" = "0 0" ] \
  || { echo "FAIL chat rows reached core"; exit 1; }
[ "$(val scd_b1_star "SELECT count(*) FROM chat.meta") $(val scd_b1_star "SELECT count(*) FROM chat.users")" = "0 0" ] \
  || { echo "FAIL chat rows reached star"; exit 1; }
echo "ok chat rows written on main stay on main"

say "chat: a sealed login on main finishes on star and the bind reaches every node"
val postgres "SELECT count(*) FROM pg_catalog.pg_roles WHERE rolname = 'chat'" | grep -q 1 || sql -d postgres -c "CREATE ROLE chat LOGIN IN ROLE scd"
sql -d scd_b1_main -c "SET ROLE chat" \
  -c "INSERT INTO public.login_requests (id, state, code_verifier, status, expires_at, chat_bind) VALUES ('00000000-0000-0000-0000-0000000c4a01', 'c4a-state', 'c4a-verifier', 'pending', now() + interval '15 minutes', '\\xb1b1')"
wait_for scd_b1_star "SELECT chat_bind = '\\xb1b1' FROM public.login_requests WHERE id = '00000000-0000-0000-0000-0000000c4a01'" "star has the login request with its bind"
sql -d scd_b1_star -f - <<'SQL'
BEGIN;
INSERT INTO public.sessions (id, chat_bind)
SELECT '00000000-0000-0000-0000-0000000c4a11', chat_bind FROM public.login_requests WHERE id = '00000000-0000-0000-0000-0000000c4a01';
UPDATE public.login_requests SET status = 'completed', result_session_id = '00000000-0000-0000-0000-0000000c4a11' WHERE id = '00000000-0000-0000-0000-0000000c4a01';
COMMIT;
SQL
wait_for scd_b1_main "SELECT r.status = 'completed' AND r.chat_bind = '\\xb1b1' AND s.chat_bind = '\\xb1b1' FROM public.login_requests r JOIN public.sessions s ON s.id = r.result_session_id WHERE r.id = '00000000-0000-0000-0000-0000000c4a01'" "main sees the completed login and the bound session"
same_on_all "SELECT encode(chat_bind, 'hex') || ' ' || bind_v || ' ' || row_v FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000c4a11'" "the bound session is equal on all nodes"
same_on_all "SELECT encode(chat_bind, 'hex') || ' ' || status || ' ' || row_v FROM public.login_requests WHERE id = '00000000-0000-0000-0000-0000000c4a01'" "the login request is equal on all nodes"

say "chat: a session touch on star does not put back an older bind"
sql -d scd_b1_main -c "INSERT INTO public.sessions (id) VALUES ('00000000-0000-0000-0000-0000000c4a12')"
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000c4a12')" "an unbound session on star"
pause star star_from_core
sql -d scd_b1_main -c "SET ROLE chat" -c "UPDATE public.sessions SET chat_bind = COALESCE('\\xb2b2'::bytea, chat_bind), updated_at = now() WHERE id = '00000000-0000-0000-0000-0000000c4a12'"
wait_for scd_b1_core "SELECT chat_bind = '\\xb2b2' FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000c4a12'" "core has main's bind"
main_v="$(val scd_b1_core "SELECT row_v FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000c4a12'")"
sql -d scd_b1_star -c "UPDATE public.sessions SET chat_bind = COALESCE(NULL, chat_bind), updated_at = '2030-01-01' WHERE id = '00000000-0000-0000-0000-0000000c4a12'"
touch_v="$(val scd_b1_star "SELECT row_v FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000c4a12'")"
[ "$touch_v" -gt "$main_v" ] || { echo "FAIL star's touch should carry the newer row version: $touch_v <= $main_v"; exit 1; }
echo "ok star's touch carries a newer row version than main's bind"
resume star star_from_core
settle ec04 ec05 ec06
same_on_all "SELECT encode(chat_bind, 'hex') || ' ' || updated_at || ' ' || bind_v || ' ' || row_v FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000c4a12'" "the session is equal on all nodes"
[ "$(val scd_b1_star "SELECT encode(chat_bind, 'hex') || ' ' || updated_at FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000c4a12'")" = "b2b2 2030-01-01 00:00:00" ] \
  || { echo "FAIL star's touch and main's bind should both survive"; exit 1; }
echo "ok star's newer touch wins the row group and main's bind wins the bind group"

say "chat: a pull link keeps its bind through a claim on star"
sql -d scd_b1_main -c "SET ROLE chat" \
  -c "INSERT INTO public.link_requests (id, claim_token, mode, status, expires_at, chat_bind) VALUES ('00000000-0000-0000-0000-0000000c4a21', 'c4a-claim', 'pull', 'pending', now() + interval '5 minutes', '\\xb3b3')"
wait_for scd_b1_star "SELECT chat_bind = '\\xb3b3' FROM public.link_requests WHERE id = '00000000-0000-0000-0000-0000000c4a21'" "star has the link with its bind"
sql -d scd_b1_star -f - <<'SQL'
BEGIN;
INSERT INTO public.sessions (id, chat_bind) VALUES ('00000000-0000-0000-0000-0000000c4a22', '\xb3b3');
UPDATE public.link_requests SET status = 'claimed', source_session_id = '00000000-0000-0000-0000-0000000c4a11', target_session_id = '00000000-0000-0000-0000-0000000c4a22' WHERE id = '00000000-0000-0000-0000-0000000c4a21';
COMMIT;
SQL
wait_for scd_b1_main "SELECT l.status = 'claimed' AND l.chat_bind = '\\xb3b3' AND s.chat_bind = '\\xb3b3' FROM public.link_requests l JOIN public.sessions s ON s.id = l.target_session_id WHERE l.id = '00000000-0000-0000-0000-0000000c4a21'" "main sees the claim, the link bind and the target session bind"
same_on_all "SELECT encode(chat_bind, 'hex') || ' ' || status || ' ' || bind_v || ' ' || row_v FROM public.link_requests WHERE id = '00000000-0000-0000-0000-0000000c4a21'" "the link request is equal on all nodes"

say "chat: a logout deletes the bound session everywhere"
sql -d scd_b1_main -c "SET ROLE chat" -c "DELETE FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000c4a11'"
settle ec07 ec08 ec09
same_on_all "SELECT count(*) FROM public.sessions WHERE id = '00000000-0000-0000-0000-0000000c4a11'" "the bound session is gone on all nodes (0 rows)"
same_on_all "SELECT count(*) FROM public.repl_tombstones WHERE tbl = 'sessions' AND pk = '{\"id\": \"00000000-0000-0000-0000-0000000c4a11\"}'" "its tombstone is on all nodes"
