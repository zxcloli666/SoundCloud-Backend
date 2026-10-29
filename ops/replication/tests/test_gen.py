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
        self.assertEqual(triggers("soundcloud_connections"), ["zz_guard_result", "zz_merge", "zz_purge_guard", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("sessions"), ["zz_merge", "zz_tomb", "zz_tomb_check", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("listening_history"), ["zz_skip", "zz_tomb", "zz_tomb_check", "zzz_suppress"])
        self.assertEqual(triggers("cdn_tracks"), ["zz_merge", "zz_ver", "zzz_suppress"])
        self.assertEqual(triggers("entity_reads"), ["zz_max", "zzz_suppress"])
        self.assertEqual(triggers("cmd_outbox"), [])
        self.assertEqual(triggers("cluster_bandit_stats"), [])

    def test_replica_triggers_are_enabled_for_replica(self):
        text = self.files["triggers.sql"]
        for name in ("zz_merge", "zz_owner", "zz_skip", "zz_tomb_check", "zz_purge_guard", "zz_max", "zzz_suppress"):
            created = len(re.findall(r"CREATE OR REPLACE TRIGGER %s " % name, text))
            enabled = len(re.findall(r"ENABLE REPLICA TRIGGER %s;" % name, text))
            self.assertEqual(created, enabled, name)
        for name in ("zz_ver", "zz_tomb", "zz_guard"):
            self.assertNotIn("ENABLE REPLICA TRIGGER %s;" % name, text)

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
