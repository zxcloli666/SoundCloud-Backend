import os
import subprocess
import sys
import tempfile
import unittest

import support
import check_ownership
import model
from sqltext import analyze


def stmt(service, sql, path="api/queries/x.sql", used_by=None, test=False, file_tables=()):
    o = {
        "path": path,
        "line": None,
        "service": service,
        "used_by": used_by or [service],
        "source": "sql_file",
        "dynamic": "⟦" in sql,
        "test": test,
        "fn": None,
        "sql": sql,
        "file_tables": list(file_tables),
    }
    o.update(analyze(sql, dynamic=o["dynamic"]))
    return o


def rules(statements, m=None):
    return sorted(r[0] for r in check_ownership.code_violations(m or model.load(), statements))


class CodeRules(unittest.TestCase):
    def test_api_write_to_c(self):
        self.assertEqual(rules([stmt("api", "UPDATE tracks SET title = $1 WHERE id = $2")]), ["api-write-c"])

    def test_streaming_write_to_c(self):
        self.assertEqual(rules([stmt("streaming", "UPDATE tracks SET title = $1", "streaming/queries/x.sql")]), ["api-write-c"])

    def test_utils_used_by_api_counts_as_api(self):
        o = stmt("utils/catalog-ingest", "INSERT INTO users (urn) VALUES ($1)", "utils/catalog-ingest/queries/x.sql", ["jobs", "api", "utils/catalog-ingest"])
        self.assertEqual(rules([o]), ["api-write-c"])

    def test_core_only_code_is_exempt(self):
        o = stmt("streaming", "UPDATE tracks SET title = $1", "streaming/src/db/postgres/upgrade.rs")
        self.assertEqual(rules([o]), [])

    def test_jobs_write_to_c_is_fine(self):
        self.assertEqual(rules([stmt("jobs", "UPDATE tracks SET title = $1", "jobs/queries/x.sql")]), [])

    def test_tests_are_ignored(self):
        self.assertEqual(rules([stmt("api", "UPDATE tracks SET title = $1", test=True)]), [])

    def test_api_write_to_p(self):
        self.assertEqual(rules([stmt("api", "INSERT INTO background_jobs (kind) VALUES ($1)")]), ["api-write-p"])

    def test_api_delete_on_m(self):
        self.assertEqual(rules([stmt("api", "DELETE FROM user_likes_tracks WHERE user_id = $1")]), ["api-delete-shared"])

    def test_core_delete_on_m_needs_the_receipt_function(self):
        self.assertEqual(rules([stmt("jobs", "DELETE FROM user_likes_tracks WHERE user_id = $1", "jobs/queries/x.sql")]), ["core-delete-m"])
        receipt = "SELECT public.scd_purge_intent('user_likes_tracks', $1)"
        self.assertEqual(rules([stmt("jobs", receipt, "jobs/queries/x.sql")]), [])

    def test_api_delete_on_tombstone_a_is_fine(self):
        self.assertEqual(rules([stmt("api", "DELETE FROM sessions WHERE id = $1")]), [])

    def test_api_delete_on_non_tombstone_a(self):
        self.assertEqual(rules([stmt("api", "DELETE FROM cdn_tracks WHERE id = $1")]), ["api-delete-shared"])

    def test_insert_only_a_updated(self):
        self.assertEqual(rules([stmt("jobs", "UPDATE user_events SET weight = 2", "jobs/queries/x.sql")]), ["a-insert-only-updated"])
        upsert = "INSERT INTO user_events (id) VALUES ($1) ON CONFLICT (id) DO UPDATE SET weight = 2"
        self.assertEqual(rules([stmt("jobs", upsert, "jobs/queries/x.sql")]), ["a-insert-only-updated"])
        self.assertEqual(rules([stmt("jobs", "INSERT INTO user_events (id) VALUES ($1) ON CONFLICT DO NOTHING", "jobs/queries/x.sql")]), [])

    def test_clock(self):
        self.assertEqual(rules([stmt("jobs", "SELECT nextval('catalog_metadata_clock')", "jobs/queries/x.sql")]), ["clock"])

    def test_unknown_kind(self):
        self.assertEqual(rules([stmt("api", "SELECT public.scd_wt('track.edit', $1)")]), [])
        self.assertEqual(rules([stmt("api", "SELECT public.scd_wt('track.nope', $1)")]), ["unknown-kind"])

    def test_dynamic_table_resolves_through_file_literals(self):
        o = stmt("api", "UPDATE ⟦table⟧ SET x = 1", "api/src/a.rs", file_tables=["tracks", "not_a_table"])
        self.assertEqual(rules([o]), ["api-write-c"])


class KindRules(unittest.TestCase):
    def kinds(self, body, name="track_edit"):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "9001_kind.sql"), "w") as f:
                f.write("CREATE OR REPLACE FUNCTION public.scd_cmd_%s(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$\nBEGIN\n%s\nEND\n$f$;\n" % (name, body))
            return sorted(r[0] for r in check_ownership.kind_violations(model.load(), d))

    def test_clean_kind(self):
        self.assertEqual(self.kinds("UPDATE public.tracks SET title = args->>'title' WHERE id = (args->>'id')::uuid; RETURN '[]';"), [])

    def test_skip_locked(self):
        self.assertIn("kind-skip-locked", self.kinds("PERFORM 1 FROM public.tracks FOR UPDATE SKIP LOCKED;"))

    def test_for_update(self):
        self.assertEqual(self.kinds("PERFORM 1 FROM public.tracks WHERE id = (args->>'id')::uuid FOR UPDATE;"), ["kind-for-update"])

    def test_limit(self):
        self.assertEqual(self.kinds("PERFORM 1 FROM public.tracks LIMIT 10;"), ["kind-limit"])

    def test_touch_outside_the_list(self):
        self.assertEqual(self.kinds("UPDATE public.users SET username = 'x';"), ["kind-touch"])

    def test_unknown_kind_function(self):
        self.assertEqual(self.kinds("RETURN '[]';", "nope_kind"), ["kind-unknown"])


class Baseline(unittest.TestCase):
    def run_check(self, root, baseline, *extra):
        cmd = [sys.executable, os.path.join(support.REPL, "check_ownership.py"), "--root", root, "--baseline", baseline] + list(extra)
        return subprocess.run(cmd, capture_output=True, text=True)

    def test_ratchet(self):
        with tempfile.TemporaryDirectory() as root:
            os.makedirs(os.path.join(root, "utils"))
            os.makedirs(os.path.join(root, "api", "queries", "t"))
            q = os.path.join(root, "api", "queries", "t", "edit.sql")
            with open(q, "w") as f:
                f.write("UPDATE tracks SET title = $1 WHERE id = $2\n")
            baseline = os.path.join(root, "baseline.txt")
            r = self.run_check(root, baseline)
            self.assertEqual(r.returncode, 1, r.stdout)
            self.assertIn("api-write-c: api/queries/t/edit.sql tracks", r.stdout)
            self.assertEqual(self.run_check(root, baseline, "--update-baseline").returncode, 0)
            self.assertEqual(self.run_check(root, baseline).returncode, 0)
            os.remove(q)
            r = self.run_check(root, baseline)
            self.assertEqual(r.returncode, 1, r.stdout)
            self.assertIn("fixed, remove from baseline.txt: api-write-c|api/queries/t/edit.sql|tracks", r.stdout)


if __name__ == "__main__":
    unittest.main()
