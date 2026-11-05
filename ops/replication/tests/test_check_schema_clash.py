import os
import tempfile
import unittest

import support
import check_schema_clash

NAMES = {"users"}


def found(sql, dynamic=False):
    return check_schema_clash.references(sql, NAMES, dynamic)


class References(unittest.TestCase):
    def test_from_and_join(self):
        self.assertEqual(found("SELECT 1 FROM users u"), [(1, "users")])
        self.assertEqual(found("SELECT 1\nFROM chat.members m\nLEFT JOIN users u ON u.id = m.id"), [(3, "users")])
        self.assertEqual(found('SELECT 1 FROM tracks t, "users" u'), [(1, "users")])

    def test_writes(self):
        self.assertEqual(found("INSERT INTO users (id) VALUES (1)"), [(1, "users")])
        self.assertEqual(found("UPDATE users SET a = 1"), [(1, "users")])
        self.assertEqual(found("DELETE FROM users WHERE a = 1"), [(1, "users")])
        self.assertEqual(found("MERGE INTO users USING src ON true WHEN MATCHED THEN DO NOTHING"), [(1, "users")])
        self.assertEqual(found("DELETE FROM chat.members m USING users u WHERE u.id = m.id"), [(1, "users")])
        self.assertEqual(found("LOCK TABLE users"), [(1, "users")])

    def test_qualified_passes(self):
        self.assertEqual(found("SELECT 1 FROM public.users u JOIN chat.users c ON c.id = u.id"), [])
        self.assertEqual(found('SELECT 1 FROM "public"."users"'), [])
        self.assertEqual(found("INSERT INTO chat.users (id) VALUES (1)"), [])

    def test_same_named_cte_passes(self):
        self.assertEqual(found("WITH users AS (SELECT 1 AS id) SELECT * FROM users"), [])
        self.assertEqual(found("WITH a AS (SELECT 1), users AS (SELECT 2) SELECT * FROM a JOIN users ON true"), [])

    def test_comment_and_literal_pass(self):
        self.assertEqual(found("SELECT 1 -- FROM users\n"), [])
        self.assertEqual(found("SELECT 1 /* JOIN users */"), [])
        self.assertEqual(found("SELECT 'FROM users', $1"), [])

    def test_columns_and_functions_pass(self):
        self.assertEqual(found("SELECT users.id FROM public.users"), [])
        self.assertEqual(found("SELECT extract(epoch FROM users) FROM public.tracks"), [])

    def test_placeholder_sql(self):
        self.assertEqual(found("SELECT 1 FROM {table} JOIN users u ON true", True), [(1, "users")])


class Tree(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        root = self.dir.name
        os.makedirs(os.path.join(root, "api", "migrations"))
        os.makedirs(os.path.join(root, "ops", "replication"))
        os.makedirs(os.path.join(root, "utils"))
        os.makedirs(os.path.join(root, "api", "queries", "chat"))
        with open(os.path.join(root, "ops", "replication", "schema.json"), "w") as f:
            f.write('{"core": {"users": {}, "tracks": {}}, "ops": {}}')
        self.root = root

    def tearDown(self):
        self.dir.cleanup()

    def write(self, rel, text):
        with open(os.path.join(self.root, rel), "w") as f:
            f.write(text)

    def test_names_come_from_migrations(self):
        self.write(
            "api/migrations/0001.sql",
            "CREATE TABLE chat.users (id int);\nCREATE TABLE chat.members (id int);\nCREATE TABLE IF NOT EXISTS chat.tracks (id int);\n"
            "CREATE VIEW chat.v AS SELECT 1;\nDROP TABLE chat.tracks;\n",
        )
        self.assertEqual(check_schema_clash.clashing(self.root), ["users"])

    def test_public_view_clashes(self):
        self.write("api/migrations/0001.sql", "CREATE VIEW public.v AS SELECT 1;\nCREATE VIEW chat.v AS SELECT 2;\n")
        self.assertEqual(check_schema_clash.clashing(self.root), ["v"])

    def test_scan_reports_file_and_line(self):
        self.write("api/queries/chat/a.sql", "SELECT 1\nFROM users\n")
        self.write("api/queries/chat/b.sql", "SELECT 1\nFROM public.users\n")
        self.assertEqual(check_schema_clash.scan(self.root, NAMES), [("api/queries/chat/a.sql", 2, "users")])


if __name__ == "__main__":
    unittest.main()
