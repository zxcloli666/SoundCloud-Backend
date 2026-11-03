import contextlib
import io
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

import support
import gen
import gen_install
import model
import sqlbits


QUALIFIED_AFTER = re.compile(r"\b(from|join|into|update|table|on|function|exists|references|trigger)\s+([a-z_][a-z0-9_.]*)", re.I)


def strip_literals(text):
    return re.sub(r"E?'(?:[^']|'')*'", "''", text)


class Generated(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.m = model.load()
        cls.files = gen.render(cls.m)
        cls.install = gen_install.render(cls.m)

    def test_deterministic(self):
        self.assertEqual(gen.render(model.load()), self.files)

    def test_checked_in_output_is_current(self):
        self.assertEqual(gen.check(self.files, gen.OUT), 0)

    def test_drift_is_detected(self):
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stderr(io.StringIO()):
            out = os.path.join(d, "generated")
            shutil.copytree(gen.OUT, out)
            self.assertEqual(gen.check(self.files, out), 0)
            with open(os.path.join(out, "triggers.sql"), "a") as f:
                f.write("\n")
            self.assertEqual(gen.check(self.files, out), 1)
            shutil.rmtree(out)
            shutil.copytree(gen.OUT, out)
            open(os.path.join(out, "extra.sql"), "w").close()
            self.assertEqual(gen.check(self.files, out), 1)

    def test_ascii_only(self):
        for name, text in self.files.items():
            self.assertTrue(all(c == "\n" or 32 <= ord(c) < 127 for c in text), name)

    def test_public_objects_are_schema_qualified(self):
        names = set(self.m.tables)
        for name, text in self.files.items():
            for word, obj in QUALIFIED_AFTER.findall(strip_literals(text)):
                bare = obj.split(".")[-1]
                if obj.startswith(("public.", "pg_catalog.", "pg_temp.")) or "." in obj:
                    continue
                self.assertFalse(bare in names or bare.startswith("scd_"), "%s: unqualified %s %s" % (name, word, obj))
            for call in re.findall(r"(?<![\w.])(scd_\w+)\s*\(", strip_literals(text)):
                self.fail("%s: unqualified call %s" % (name, call))

    def test_triggers_per_class(self):
        text = self.files["triggers.sql"]

        def triggers(table):
            return sorted(set(re.findall(r"CREATE OR REPLACE TRIGGER (\w+) [^;]*? ON public\.%s " % table, text)))

        self.assertEqual(triggers("tracks"), ["zz_guard", "zz_owner", "zzz_suppress"])
        self.assertEqual(triggers("artists"), ["zz_core_col", "zz_guard", "zz_owner", "zzz_suppress"])
        self.assertEqual(triggers("background_jobs"), ["zz_guard"])
        self.assertEqual(triggers("user_likes_tracks"), ["zz_guard_intent", "zz_guard_result", "zz_merge", "zz_purge_guard", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("soundcloud_connections"),
                         ["zz_fk", "zz_guard_result", "zz_merge", "zz_orphan", "zz_purge_guard", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("oauth_apps"), ["zz_fk", "zz_merge", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("sessions"), ["zz_merge", "zz_orphan", "zz_tomb", "zz_tomb_check", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("listening_history"), ["zz_skip", "zz_tomb", "zz_tomb_check", "zzz_suppress"])
        self.assertEqual(triggers("disliked_tracks"), ["zz_merge", "zz_tomb", "zz_tomb_check", "zz_tomb_guard", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("repl_tombstones"), ["zz_erase", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("cdn_tracks"), ["zz_merge", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("entity_reads"), ["zz_max", "zzz_suppress"])
        self.assertEqual(triggers("cmd_outbox"), [])
        self.assertEqual(triggers("cluster_bandit_stats"), [])

    def test_replica_triggers_are_enabled_for_replica(self):
        text = self.files["triggers.sql"]
        for name in ("zz_merge", "zz_owner", "zz_skip", "zz_tomb_check", "zz_tomb_guard", "zz_erase", "zz_purge_guard", "zz_max", "zz_fk",
                     "zz_orphan", "zzz_suppress"):
            created = len(re.findall(r"CREATE OR REPLACE TRIGGER %s " % name, text))
            enabled = len(re.findall(r"ENABLE REPLICA TRIGGER %s;" % name, text))
            self.assertEqual(created, enabled, name)
        for name in ("zz_ver", "zz_tomb", "zz_guard", "zz_core_col"):
            self.assertNotIn("ENABLE REPLICA TRIGGER %s;" % name, text)

    def test_replica_triggers_are_dormant_in_legacy(self):
        text = self.files["triggers.sql"]
        for m in re.finditer(r"CREATE OR REPLACE TRIGGER (zz\w+) ([^;]*);\nALTER TABLE \S+ ENABLE REPLICA TRIGGER", text):
            self.assertIn("public.scd_mode() = 'split'", m.group(2), m.group(1))

    def test_intent_guard_covers_inserts(self):
        text = self.files["triggers.sql"]
        self.assertRegex(text, r"CREATE OR REPLACE TRIGGER zz_guard_intent BEFORE INSERT OR UPDATE OF [^;]*wanted_state[^;]* ON public\.user_likes_tracks FOR EACH STATEMENT")
        self.assertRegex(text, r"CREATE OR REPLACE TRIGGER zz_guard_result BEFORE UPDATE OF [^;]* ON public\.user_likes_tracks FOR EACH STATEMENT")

    def test_core_groups_carry_no_version(self):
        for name in ("triggers.sql", "row_hash.sql", "publications.sql"):
            self.assertNotIn("result_v", self.files[name])
            self.assertNotIn("refresh_v", self.files[name])

    def test_merge_keeps_core_results_on_core_and_takes_them_on_api(self):
        text = self.files["triggers.sql"]
        body = text[text.index("FUNCTION public.scd_merge_user_likes_tracks()"):text.index("TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_likes_tracks")]
        self.assertIn("IF NEW.wanted_v < OLD.wanted_v OR (core AND NEW.wanted_v = OLD.wanted_v) THEN", body)
        self.assertIn("        IF core THEN\n            NEW.progress := OLD.progress;", body)
        self.assertIn("progress = CASE WHEN core THEN o.progress ELSE NEW.progress END", body)
        self.assertIn("created_at = CASE WHEN ((NEW.wanted_v & 15) <> public.scd_node_ord() OR NEW.wanted_v = o.wanted_v) THEN least(o.created_at, NEW.created_at)", body)
        self.assertIn("pg_catalog.set_config('scd.merging', 'on', true)", body)

    def test_optimistic_m_kind_holds_core_images_and_keeps_its_version(self):
        text = self.files["triggers.sql"]
        merge = text[text.index("FUNCTION public.scd_merge_soundcloud_connections()"):text.index("TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.soundcloud_connections")]
        hold = merge.index("    IF NOT core THEN\n        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'soundcloud_connections' AND p.pk = jsonb_build_object('id', NEW.id)) THEN\n            RETURN NULL;")
        self.assertLess(hold, merge.index("pv := "))
        self.assertNotIn("cmd_pending", text[text.index("FUNCTION public.scd_merge_user_likes_tracks()"):text.index("TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_likes_tracks")])
        self.assertRegex(text, r"zz_guard_result [^;]* ON public\.soundcloud_connections [^;]*scd_guard\('public\.scd_guard_result_soundcloud_connections', 'cmd', 'migration', 'repl'\)")
        self.assertRegex(text, r"zz_guard_result [^;]* ON public\.user_likes_tracks [^;]*scd_guard\('public\.scd_guard_result_user_likes_tracks', 'migration', 'repl'\)")
        ver = text[text.index("FUNCTION public.scd_ver_soundcloud_connections()"):text.index("TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.soundcloud_connections")]
        self.assertIn("AND NOT (coalesce(current_setting('scd.cmd', true), '') <> '' AND NEW.token_v > greatest(OLD.token_v, ", ver)

    def test_replica_inserts_merge_inside_the_trigger(self):
        text = self.files["triggers.sql"]
        for fn in ("scd_merge_disliked_tracks", "scd_merge_user_likes_tracks", "scd_max_sc_track_counters",
                   "scd_owner_playlists", "scd_skip_user_events"):
            body = text[text.index("FUNCTION public.%s()" % fn):]
            body = body[:body.index("$f$;")]
            self.assertIn("IF current_setting('scd.merging', true) = 'on' THEN", body, fn)
            self.assertIn("ON CONFLICT DO NOTHING;", body, fn)
            if fn != "scd_skip_user_events":
                self.assertIn("FOR attempt IN 1..3 LOOP", body, fn)
                self.assertIn("USING ERRCODE = 'unique_violation'", body, fn)
        merge = text[text.index("FUNCTION public.scd_merge_disliked_tracks()"):]
        merge = merge[:merge.index("$f$;")]
        self.assertEqual(merge.count("GET DIAGNOSTICS n = ROW_COUNT;\n                PERFORM pg_catalog.set_config('scd.merging', '', true);\n"
                                     "                IF n = 0 THEN\n                    CONTINUE;"), 2)
        owner = text[text.index("FUNCTION public.scd_owner_playlists()"):]
        owner = owner[:owner.index("$f$;")]
        locked = owner.index("    FOR attempt IN 1..3 LOOP\n        PERFORM 1 FROM public.playlists t WHERE t.urn = NEW.urn FOR UPDATE;\n"
                             "        held := FOUND;\n        IF EXISTS (SELECT 1 FROM public.cmd_pending p")
        self.assertEqual(owner.count("FROM public.cmd_pending p"), 6)
        self.assertNotIn("IF FOUND", owner)
        self.assertLess(owner.index("        IF held THEN\n            UPDATE public.playlists t SET"), owner.index("ON CONFLICT DO NOTHING;"))
        self.assertLess(owner.index("IF TG_OP = 'UPDATE' THEN\n        IF NEW.urn IS DISTINCT FROM OLD.urn THEN"), locked)
        self.assertIn("            UPDATE public.playlists t SET last_read_at = NEW.last_read_at WHERE t.urn = NEW.urn AND t.last_read_at IS DISTINCT FROM NEW.last_read_at;\n"
                      "            PERFORM pg_catalog.set_config('scd.merging', '', true);\n            RETURN NULL;", owner)
        self.assertIn("                IF NEW.last_read_at IS DISTINCT FROM OLD.last_read_at THEN\n                    NEW.artwork_url := OLD.artwork_url;", owner)

    def test_owner_holds_every_incoming_change_of_a_pending_key(self):
        text = self.files["triggers.sql"]
        owner = text[text.index("FUNCTION public.scd_owner_playlist_track_projection()"):]
        owner = owner[:owner.index("$f$;")]
        old = "p.tbl = 'playlist_track_projection' AND p.pk = jsonb_build_object('playlist_urn', OLD.playlist_urn, 'position', OLD.\"position\")"
        new = "p.tbl = 'playlist_track_projection' AND p.pk = jsonb_build_object('playlist_urn', NEW.playlist_urn, 'position', NEW.\"position\")"
        self.assertIn("IF TG_OP = 'DELETE' THEN\n        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE %s) THEN\n"
                      "            RETURN NULL;" % old, owner)
        self.assertIn("IF (NEW.playlist_urn, NEW.\"position\") IS DISTINCT FROM (OLD.playlist_urn, OLD.\"position\") THEN\n"
                      "            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE %s) THEN\n"
                      "                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE %s) THEN\n"
                      "                    INSERT INTO public.playlist_track_projection (playlist_urn, \"position\", sc_track_id) VALUES (NEW.playlist_urn, NEW.\"position\", NEW.sc_track_id);\n"
                      "                END IF;\n                RETURN NULL;\n            END IF;\n"
                      "            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE %s) THEN\n"
                      "                DELETE FROM public.playlist_track_projection t WHERE (t.playlist_urn, t.\"position\") = (OLD.playlist_urn, OLD.\"position\");\n"
                      "                RETURN NULL;" % (old, new, new), owner)
        self.assertIn("CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.playlist_track_projection", text)
        self.assertIn("SELECT $1 = ANY (ARRAY['artist_sc_accounts', 'artists', 'playlist_membership_operations', 'playlist_track_projection', "
                      "'track_comments', 'wanted_tracks']::text[])", self.files["registry.sql"])

    def test_insert_only_least_merges_down_only_on_a_key_conflict(self):
        text = self.files["triggers.sql"]
        skip = text[text.index("FUNCTION public.scd_skip_user_events()"):]
        skip = skip[:skip.index("$f$;")]
        self.assertEqual(skip.count("FROM public.user_events t WHERE t.id = NEW.id"), 1)
        self.assertIn("IF TG_OP = 'UPDATE' THEN\n        NEW.created_at := least(OLD.created_at, NEW.created_at);\n        RETURN NEW;", skip)
        self.assertIn("    IF FOUND THEN\n        IF NEW.created_at < o.created_at THEN\n", skip)
        self.assertIn("UPDATE public.user_events t SET created_at = least(t.created_at, NEW.created_at) WHERE t.id = NEW.id;", skip)
        self.assertIn("CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT OR UPDATE ON public.user_events", text)
        self.assertIn("CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT ON public.diag_events", text)

    def test_purged_m_rows_never_come_back(self):
        text = self.files["triggers.sql"]

        def body(fn):
            out = text[text.index("FUNCTION public.%s(" % fn):]
            return out[:out.index("$f$;")]

        merge = body("scd_merge_user_likes_tracks")
        lookup = "pv := (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'user_likes_tracks'"
        self.assertLess(merge.index(lookup), merge.index("IF TG_OP = 'UPDATE' THEN"))
        self.assertLess(merge.index("IF current_setting('scd.merging', true) = 'on' THEN"), merge.index(lookup))
        self.assertIn("    IF pv IS NOT NULL AND NEW.wanted_v <= pv THEN\n"
                      "        PERFORM nextval('public.scd_purge_block_user_likes_tracks'::regclass);\n"
                      "        RETURN NULL;\n", merge)
        ver = body("scd_ver_user_likes_tracks")
        self.assertEqual(ver.count("FROM public.repl_purges x WHERE x.tbl = 'user_likes_tracks'"), 2)
        self.assertIn("NEW.wanted_v := public.scd_hlc(greatest(OLD.wanted_v, NEW.wanted_v, (SELECT max(x.purged_v)", ver)
        for fn in ("scd_merge_disliked_tracks", "scd_ver_disliked_tracks"):
            self.assertNotIn("repl_purges", body(fn), fn)
        for tbl in ("soundcloud_connections", "sync_queue", "user_followings", "user_likes_playlists", "user_likes_tracks"):
            self.assertIn("('purge_block', '%s', 'scd_purge_block_%s')" % (tbl, tbl), text)

    def test_tombstones_are_versioned_deletes(self):
        text = self.files["triggers.sql"]

        def body(fn):
            out = text[text.index("FUNCTION public.%s(" % fn):]
            return out[:out.index("$f$;")]

        self.assertIn("CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.disliked_tracks", text)
        self.assertNotIn("AFTER DELETE", text)
        self.assertIn("RETURN OLD;", body("scd_tomb_disliked_tracks"))
        self.assertIn("VALUES ('disliked_tracks', jsonb_build_object('id', OLD.id), OLD.row_v, now());", body("scd_tomb_disliked_tracks"))
        self.assertNotIn("ON CONFLICT", body("scd_tomb_disliked_tracks"))
        newest = "tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'disliked_tracks' AND x.pk = jsonb_build_object('id', NEW.id));"
        self.assertIn(newest, body("scd_tomb_check_disliked_tracks"))
        self.assertIn("IF tv IS NOT NULL AND NEW.row_v <= tv THEN", body("scd_tomb_check_disliked_tracks"))
        self.assertIn("VALUES ('repl_tombstones', jsonb_build_object('tbl', 'disliked_tracks', 'pk', jsonb_build_object('id', NEW.id), 'v', tv))",
                      body("scd_tomb_check_disliked_tracks"))
        self.assertIn("IF tv IS NOT NULL AND NEW.row_v <= tv THEN", body("scd_merge_disliked_tracks"))
        self.assertIn("IF tv IS NOT NULL THEN\n        RETURN NULL;", body("scd_tomb_check_sessions"))
        self.assertNotIn("repl_resend", body("scd_tomb_check_sessions"))
        self.assertIn("AND t.row_v <= upto;", body("scd_erase_disliked_tracks"))
        self.assertIn("VALUES ('disliked_tracks', k)", body("scd_erase_disliked_tracks"))
        self.assertIn("DELETE FROM public.sessions t WHERE t.id = (k ->> 'id')::uuid;", body("scd_erase_sessions"))
        self.assertNotIn("repl_resend", body("scd_erase_sessions"))
        self.assertIn("IF tv IS NULL OR OLD.row_v <= tv THEN", body("scd_tomb_guard_disliked_tracks"))
        self.assertNotIn("FUNCTION public.scd_tomb_guard_sessions()", text)
        self.assertIn("WHEN 'disliked_tracks' THEN\n            PERFORM public.scd_erase_disliked_tracks(k, upto);", body("scd_erase"))
        self.assertIn("ELSE\n            NULL;", body("scd_erase"))
        self.assertIn("CREATE OR REPLACE TRIGGER zz_erase BEFORE INSERT OR UPDATE ON public.repl_tombstones", text)
        arrive = body("scd_erase_repl_tombstones")
        locked = arrive.index("PERFORM public.scd_tomb_lock(NEW.tbl, NEW.pk);")
        written = arrive.index("INSERT INTO public.repl_tombstones (tbl, pk, v, at) VALUES (NEW.tbl, NEW.pk, NEW.v, NEW.at) ON CONFLICT DO NOTHING;")
        erased = arrive.index("PERFORM public.scd_erase(NEW.tbl, NEW.pk, NEW.v);")
        self.assertLess(locked, written)
        self.assertLess(written, erased)
        self.assertIn("RETURN NULL;", arrive)
        self.assertIn("WHEN 'disliked_tracks' THEN\n            PERFORM 1 FROM public.disliked_tracks t WHERE t.id = (k ->> 'id')::uuid FOR UPDATE;",
                      body("scd_tomb_lock"))
        self.assertIn("RAISE EXCEPTION 'scd: public.repl_tombstones rows are immutable", body("scd_ver_repl_tombstones"))
        self.assertIn("NEW.v := public.scd_hlc(greatest(NEW.v, (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = NEW.tbl AND x.pk = NEW.pk)));",
                      body("scd_ver_repl_tombstones"))
        self.assertNotIn("FUNCTION public.scd_merge_repl_tombstones()", text)
        self.assertIn("DROP FUNCTION IF EXISTS public.scd_merge_repl_tombstones();", text)
        self.assertNotIn("scd_erase", body("scd_merge_disliked_tracks"))
        self.assertIn("NEW.row_v := public.scd_hlc(greatest(NEW.row_v, (SELECT max(x.v) FROM public.repl_tombstones x", body("scd_ver_disliked_tracks"))
        self.assertNotIn("repl_tombstones", body("scd_ver_sessions"))

    def test_core_columns_stay_out_of_the_hash_and_come_from_core(self):
        text = self.files["triggers.sql"]

        def body(fn):
            out = text[text.index("FUNCTION public.%s(" % fn):]
            return out[:out.index("$f$;")]

        artists = self.files["row_hash.sql"][self.files["row_hash.sql"].index("FUNCTION public.scd_row_hash_artists("):]
        artists = artists[:artists.index("$f$;")]
        self.assertNotIn("has_sc_account", artists)
        self.assertIn("t.sc_user_id", artists)
        self.assertIn("SELECT pg_catalog.to_jsonb(t) FROM public.artists t WHERE", self.files["row_json.sql"])
        self.assertIn("('artists', 'has_sc_account'),\n    ('lyrics_cache', 'fts')", self.files["registry.sql"])
        self.assertIn("CREATE OR REPLACE TRIGGER zz_core_col BEFORE UPDATE OF has_sc_account ON public.artists FOR EACH ROW "
                      "WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split')", text)
        self.assertIn("NEW.has_sc_account := OLD.has_sc_account;", body("scd_core_col_artists"))
        owner = body("scd_owner_artists")
        self.assertIn("IF NEW.has_sc_account IS DISTINCT FROM OLD.has_sc_account THEN\n                    NEW.account_walk_locked_at := OLD.account_walk_locked_at;", owner)
        self.assertNotIn("NEW.has_sc_account := OLD.has_sc_account;", owner)
        self.assertIn("UPDATE public.artists t SET has_sc_account = NEW.has_sc_account WHERE t.id = NEW.id AND t.has_sc_account IS DISTINCT FROM NEW.has_sc_account;",
                      owner)
        self.assertNotIn("has_sc_account", body("scd_owner_playlists"))

    def test_resend_all_queues_what_each_role_publishes(self):
        text = self.files["registry.sql"]
        body = text[text.index("FUNCTION public.scd_resend_all("):]
        body = body[:body.index("$f$;")]
        self.assertIn("CASE public.scd_role() WHEN 'core' THEN ARRAY['C', 'M', 'A'] ELSE ARRAY['M', 'A', 'K'] END", body)
        self.assertIn("('repl_tombstones', 'A')", body)
        self.assertNotIn("artist_colike", body)
        self.assertIn("WHEN 'repl_tombstones' THEN\n            INSERT INTO public.repl_resend (tbl, pk) SELECT 'repl_tombstones', "
                      "jsonb_build_object('tbl', t.tbl, 'pk', t.pk, 'v', t.v) FROM public.repl_tombstones t ON CONFLICT DO NOTHING;", text)

    def test_fk_guard_refuses_restrict_and_applies_actions(self):
        text = self.files["triggers.sql"]
        body = text[text.index("FUNCTION public.scd_fk_oauth_apps()"):text.index("TRIGGER zz_fk BEFORE DELETE ON public.oauth_apps")]
        self.assertIn("IF EXISTS (SELECT 1 FROM public.soundcloud_connections c WHERE c.oauth_app_id = OLD.id) THEN", body)
        self.assertIn("INSERT INTO public.repl_resend", body)
        self.assertIn("DELETE FROM public.oauth_app_tokens c WHERE c.oauth_app_id = OLD.id;", body)
        self.assertIn("UPDATE public.oauth_app_token_issuance_reservations c SET oauth_app_id = NULL", body)
        self.assertIn("WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split')", text[text.index("TRIGGER zz_fk BEFORE DELETE ON public.oauth_apps"):][:300])

    def test_purge_intent_writes_the_receipt_before_the_delete(self):
        text = self.files["purge_children.sql"]
        body = text[text.index("FUNCTION public.scd_purge_intent_user_likes_tracks(k jsonb, seen bigint)"):]
        body = body[:body.index("$f$;")]
        self.assertIn("IF public.scd_role() <> 'core' THEN", body)
        self.assertIn("SELECT t.wanted_v, jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) INTO v, key", body)
        self.assertIn("FOR UPDATE;\n    IF NOT FOUND OR v IS DISTINCT FROM seen THEN\n        RETURN false;", body)
        self.assertLess(body.index("INSERT INTO public.repl_purges"), body.index("DELETE FROM public.user_likes_tracks"))
        self.assertIn("WHEN 'user_likes_tracks' THEN\n            RETURN public.scd_purge_intent_user_likes_tracks(k, seen);", text)
        self.assertIn("FUNCTION public.scd_purge_intent(tbl text, k jsonb, seen bigint)", text)
        self.assertIn("DROP FUNCTION IF EXISTS public.scd_purge_intent(text, jsonb);", text)
        self.assertIn("DROP FUNCTION IF EXISTS public.scd_purge_intent_user_likes_tracks(jsonb);", text)

    def test_install_is_one_short_transaction_per_block(self):
        text = self.install
        self.assertTrue(text.startswith("-- no-transaction\nBEGIN;\nSET LOCAL lock_timeout = '5s';\n"))
        blocks = text[len("-- no-transaction\n"):].split("COMMIT;\n")
        self.assertEqual(blocks[-1], "")
        for b in blocks[:-1]:
            b = b.lstrip("\n")
            self.assertTrue(b.startswith("BEGIN;\nSET LOCAL lock_timeout = '5s';\n"), b[:80])
            locked = set(re.findall(r"(?:ON|ALTER TABLE) public\.(\w+) ", b))
            self.assertLessEqual(len(locked), 1, sorted(locked))
        for name in ("triggers.sql", "row_hash.sql", "row_json.sql", "purge_children.sql", "publications.sql", "registry.sql"):
            for fn in re.findall(r"CREATE OR REPLACE FUNCTION (\S+)\(", self.files[name]):
                self.assertIn("CREATE OR REPLACE FUNCTION %s(" % fn, text, fn)
        self.assertEqual(sorted(re.findall(r"CREATE OR REPLACE TRIGGER .*", self.files["triggers.sql"])),
                         sorted(re.findall(r"CREATE OR REPLACE TRIGGER .*", text)))

    def test_install_drift_is_detected(self):
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(gen.check_install(self.install, d), 1)
            with open(os.path.join(d, "0001_scd_install.sql"), "w") as f:
                f.write(self.install)
            self.assertEqual(gen.check_install(self.install, d), 0)
            with open(os.path.join(d, "0002_scd_install.sql"), "w") as f:
                f.write(self.install + "\n")
            self.assertEqual(gen.check_install(self.install, d), 1)

    def test_checked_in_install_is_current(self):
        self.assertEqual(gen.check_install(self.install, gen.MIGRATIONS), 0)

    def test_bad_yaml_fails_the_generator(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "ownership.yaml")
            with open(os.path.join(support.REPL, "ownership.yaml"), encoding="utf-8") as f:
                text = f.read()
            with open(path, "w") as f:
                f.write(text.replace("  albums: {class: C}\n", ""))
            r = subprocess.run([sys.executable, os.path.join(support.REPL, "gen.py"), "--yaml", path, "--out", d], capture_output=True, text=True)
            self.assertEqual(r.returncode, 1)
            self.assertIn("albums: table in the core schema but not in ownership.yaml", r.stderr)


class SqlBits(unittest.TestCase):
    def test_qualify(self):
        expr = "btrim(client_id, ' x'::text) || name || 'name' || t.name"
        self.assertEqual(sqlbits.qualify(expr, ["client_id", "name"], "NEW"), "btrim(NEW.client_id, ' x'::text) || NEW.name || 'name' || t.name")
        self.assertEqual(sqlbits.qualify("((action_type)::text <> 'comment'::text)", ["action_type", "text"], "t"), "((t.action_type)::text <> 'comment'::text)")

    def test_ascii_literal(self):
        self.assertEqual(sqlbits.qualify("'a\tb ''c'", [], "t"), "E'a\\u0009b\\u2028\\'c'")

    def test_keyword_columns_are_quoted(self):
        self.assertEqual(sqlbits.ref("NEW", "row"), 'NEW."row"')
        self.assertEqual(sqlbits.ref("NEW", "range"), "NEW.range")


if __name__ == "__main__":
    unittest.main()
