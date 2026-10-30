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
        self.assertEqual(triggers("background_jobs"), ["zz_guard"])
        self.assertEqual(triggers("user_likes_tracks"), ["zz_guard_intent", "zz_guard_result", "zz_merge", "zz_purge_guard", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("soundcloud_connections"),
                         ["zz_fk", "zz_guard_result", "zz_merge", "zz_orphan", "zz_purge_guard", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("oauth_apps"), ["zz_fk", "zz_merge", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("sessions"), ["zz_merge", "zz_orphan", "zz_tomb", "zz_tomb_check", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("listening_history"), ["zz_skip", "zz_tomb", "zz_tomb_check", "zzz_suppress"])
        self.assertEqual(triggers("disliked_tracks"), ["zz_merge", "zz_tomb", "zz_tomb_check", "zz_tomb_guard", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("repl_tombstones"), ["zz_erase", "zz_merge", "zz_ver", "zzz_suppress"])
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
        for name in ("zz_ver", "zz_tomb", "zz_guard"):
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
        self.assertIn("created_at = CASE WHEN (NEW.wanted_v & 15) <> public.scd_node_ord() THEN least(o.created_at, NEW.created_at)", body)
        self.assertIn("pg_catalog.set_config('scd.merging', 'on', true)", body)

    def test_replica_inserts_merge_inside_the_trigger(self):
        text = self.files["triggers.sql"]
        for fn in ("scd_merge_disliked_tracks", "scd_merge_user_likes_tracks", "scd_merge_repl_tombstones", "scd_max_sc_track_counters",
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
                             "        IF EXISTS (SELECT 1 FROM public.cmd_pending p")
        self.assertEqual(owner.count("FROM public.cmd_pending p"), 2)
        self.assertLess(owner.index("IF TG_OP = 'UPDATE' THEN\n        IF EXISTS (SELECT 1 FROM public.cmd_pending p"), locked)
        self.assertNotIn("last_read_at = NEW.last_read_at", owner)
        self.assertIn("NEW.last_read_at", owner)

    def test_tombstones_are_versioned_deletes(self):
        text = self.files["triggers.sql"]

        def body(fn):
            out = text[text.index("FUNCTION public.%s(" % fn):]
            return out[:out.index("$f$;")]

        self.assertIn("CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.disliked_tracks", text)
        self.assertNotIn("AFTER DELETE", text)
        self.assertIn("RETURN OLD;", body("scd_tomb_disliked_tracks"))
        self.assertIn("IF FOUND AND NEW.row_v <= tv THEN", body("scd_tomb_check_disliked_tracks"))
        self.assertIn("VALUES ('repl_tombstones', jsonb_build_object('tbl', 'disliked_tracks'", body("scd_tomb_check_disliked_tracks"))
        self.assertIn("IF FOUND AND NEW.row_v <= tv THEN", body("scd_merge_disliked_tracks"))
        self.assertIn("IF FOUND THEN\n        RETURN NULL;", body("scd_tomb_check_sessions"))
        self.assertNotIn("repl_resend", body("scd_tomb_check_sessions"))
        self.assertIn("AND t.row_v <= upto;", body("scd_erase_disliked_tracks"))
        self.assertIn("VALUES ('disliked_tracks', k)", body("scd_erase_disliked_tracks"))
        self.assertIn("DELETE FROM public.sessions t WHERE t.id = (k ->> 'id')::uuid;", body("scd_erase_sessions"))
        self.assertNotIn("repl_resend", body("scd_erase_sessions"))
        self.assertIn("IF NOT FOUND OR OLD.row_v <= tv THEN", body("scd_tomb_guard_disliked_tracks"))
        self.assertNotIn("FUNCTION public.scd_tomb_guard_sessions()", text)
        self.assertIn("WHEN 'disliked_tracks' THEN\n            PERFORM public.scd_erase_disliked_tracks(k, upto);", body("scd_erase"))
        self.assertIn("ELSE\n            NULL;", body("scd_erase"))
        self.assertIn("CREATE OR REPLACE TRIGGER zz_erase AFTER INSERT OR UPDATE ON public.repl_tombstones", text)
        self.assertIn("IF NEW.v <= o.v THEN\n                PERFORM public.scd_erase(NEW.tbl, NEW.pk, o.v);", body("scd_merge_repl_tombstones"))
        self.assertIn("IF NEW.v <= OLD.v THEN\n            PERFORM public.scd_erase(NEW.tbl, NEW.pk, OLD.v);", body("scd_merge_repl_tombstones"))
        self.assertNotIn("scd_erase", body("scd_merge_disliked_tracks"))
        self.assertIn("NEW.row_v := public.scd_hlc(greatest(NEW.row_v, (SELECT x.v FROM public.repl_tombstones x", body("scd_ver_disliked_tracks"))
        self.assertNotIn("repl_tombstones", body("scd_ver_sessions"))

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
