import os
import subprocess
import sys
import tempfile
import unittest

import support
import check_migrations
import model


GATED = """DO $$
BEGIN
    IF public.scd_role() = 'core' THEN
        UPDATE public.tracks SET title = btrim(title);
    END IF;
END
$$;
"""


class MigrationRules(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.m = model.load()

    def tearDown(self):
        self.dir.cleanup()

    def write(self, name, text):
        path = os.path.join(self.dir.name, name)
        with open(path, "w") as f:
            f.write(text)
        return path

    def rules(self, text, earlier=None):
        if earlier:
            self.write("9000_earlier.sql", earlier)
        path = self.write("9001_new.sql", text)
        marks = check_migrations.unpublish_marks(self.dir.name)
        return sorted(r for r, _ in check_migrations.check_file(self.m, path, marks))

    def test_plain_dml(self):
        self.assertEqual(self.rules("UPDATE public.tracks SET title = 'x';"), ["dml-outside-gate"])
        self.assertEqual(self.rules("INSERT INTO featured_items (id) VALUES (gen_random_uuid());"), ["dml-outside-gate"])
        self.assertEqual(self.rules("WITH x AS (SELECT 1) DELETE FROM public.sessions;"), ["dml-outside-gate"])

    def test_gated_dml(self):
        self.assertEqual(self.rules(GATED), [])

    def test_ungated_do_block(self):
        self.assertEqual(self.rules("DO $$ BEGIN UPDATE public.tracks SET title = 'x'; END $$;"), ["dml-outside-gate"])

    def test_function_bodies_are_not_dml(self):
        fn = "CREATE OR REPLACE FUNCTION public.f() RETURNS void LANGUAGE sql AS $f$ DELETE FROM public.sessions $f$;"
        self.assertEqual(self.rules(fn), [])

    def test_cascade(self):
        self.assertEqual(self.rules("DROP TABLE public.old_thing CASCADE;"), ["cascade"])
        self.assertEqual(self.rules("ALTER TABLE public.tracks DROP COLUMN last_read_at CASCADE;"), ["cascade"])

    def test_fk_cascade_is_fine(self):
        ddl = "CREATE TABLE public.t (id bigint PRIMARY KEY, track_id uuid REFERENCES public.tracks (id) ON DELETE CASCADE);"
        self.assertEqual(self.rules(ddl), [])

    def test_drop_published_column(self):
        self.assertEqual(self.rules("ALTER TABLE public.tracks DROP COLUMN title;"), ["drop-column"])
        self.assertEqual(self.rules("ALTER TABLE tracks DROP title;"), ["drop-column"])
        self.assertEqual(self.rules("ALTER TABLE public.tracks ADD COLUMN x int, DROP COLUMN IF EXISTS title;"), ["drop-column"])

    def test_drop_after_unpublish(self):
        earlier = "SELECT public.scd_unpublish_column('tracks', 'title');"
        self.assertEqual(self.rules("ALTER TABLE public.tracks DROP COLUMN title;", earlier), [])

    def test_unpublish_in_the_same_file_is_not_enough(self):
        text = "SELECT public.scd_unpublish_column('tracks', 'title');\nALTER TABLE public.tracks DROP COLUMN title;"
        self.assertEqual(self.rules(text), ["drop-column"])

    def test_drop_local_or_private_columns(self):
        self.assertEqual(self.rules("ALTER TABLE public.tracks DROP COLUMN last_read_at;"), [])
        self.assertEqual(self.rules("ALTER TABLE public.background_jobs DROP COLUMN kind;"), [])

    def test_alter_column_drops_are_not_column_drops(self):
        self.assertEqual(self.rules("ALTER TABLE public.tracks ALTER COLUMN title DROP DEFAULT, ALTER COLUMN title DROP NOT NULL;"), [])
        self.assertEqual(self.rules("ALTER TABLE public.tracks DROP CONSTRAINT tracks_x_check;"), [])

    def test_renames(self):
        self.assertEqual(self.rules("ALTER TABLE public.tracks RENAME COLUMN title TO name;"), ["rename-column"])
        self.assertEqual(self.rules("ALTER TABLE public.tracks RENAME TO songs;"), ["rename-table"])
        self.assertEqual(self.rules("ALTER TABLE public.tracks RENAME CONSTRAINT a TO b;"), [])

    def test_clock(self):
        self.assertEqual(self.rules("SELECT nextval('catalog_metadata_clock');"), ["clock"])

    def test_cli_only_checks_files_in_the_migrations_dir(self):
        bad = self.write("9002_bad.sql", "UPDATE public.tracks SET title = 'x';")
        cmd = [sys.executable, os.path.join(support.REPL, "check_migrations.py"), "--migrations", self.dir.name]
        self.assertEqual(subprocess.run(cmd + [bad], capture_output=True).returncode, 1)
        other = os.path.join(self.dir.name, "..")
        self.assertEqual(subprocess.run(cmd + [other, ""], capture_output=True).returncode, 0)


if __name__ == "__main__":
    unittest.main()
