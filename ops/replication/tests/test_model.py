import unittest

import model
import support
from support import col, fk, uniq


def problems(s=None, sc=None):
    return support.build(s, sc).problems()


class Model(unittest.TestCase):
    def test_real_ownership_is_clean(self):
        self.assertEqual(model.load().problems(), [])

    def test_fixture_is_clean(self):
        self.assertEqual(problems(), [])

    def assertProblem(self, text, s=None, sc=None):
        found = problems(s, sc)
        self.assertTrue(any(text in p for p in found), "%r not in %r" % (text, found))

    def test_unclassified_table(self):
        sc = support.schema()
        sc["core"]["new_table"] = support.table([col("id", "bigint", True)], [support.pk(["id"])])
        self.assertProblem("new_table: table in the core schema but not in ownership.yaml", sc=sc)

    def test_unversioned_mutable_a_table(self):
        s = support.spec()
        del s["tables"]["marks"]["groups"]
        self.assertProblem("marks: A table needs version groups", s)

    def test_undeclared_unique_key_on_a_table(self):
        s = support.spec()
        del s["tables"]["marks"]["unique"]
        self.assertProblem("unique index marks_user_item_uq must be declared as id (R6) or random", s)

    def test_r6_id_must_match_the_index(self):
        s = support.spec()
        s["tables"]["marks"]["id"]["from"] = ["user_id"]
        self.assertProblem("id from ['user_id'] does not match unique index marks_user_item_uq", s)

    def test_m_column_in_no_group(self):
        s = support.spec()
        s["tables"]["likes"]["groups"]["result"]["cols"] = ["progress"]
        self.assertProblem("likes: column synced_at is in no group", s)

    def test_m_needs_a_core_group(self):
        s = support.spec()
        del s["tables"]["likes"]["groups"]["result"]
        s["tables"]["likes"]["local"] = ["progress", "synced_at"]
        self.assertProblem("likes: M table needs a core-owned result group", s)

    def test_column_in_two_groups(self):
        s = support.spec()
        s["tables"]["likes"]["groups"]["result"]["cols"] = ["progress", "synced_at", "wanted"]
        self.assertProblem("likes: column wanted is in groups intent and result", s)

    def test_unknown_key(self):
        s = support.spec()
        s["tables"]["items"]["owner"] = "core"
        self.assertProblem("items: unknown key 'owner'", s)

    def test_fk_from_published_to_private_needs_listing(self):
        sc = support.schema()
        sc["core"]["items"]["fks"].append(fk("items_job_fkey", ["title"], "jobs_q", ["id"]))
        self.assertProblem("items: FK items_job_fkey points at P table jobs_q", sc=sc)

    def test_not_null_outside_the_list_breaks_inserts(self):
        sc = support.schema()
        sc["core"]["items"]["columns"][3] = col("last_read_at", "timestamptz", True)
        self.assertProblem("items: column last_read_at is NOT NULL without a default and not in scd_core_pub", sc=sc)

    def test_optimistic_kind_cannot_touch_m(self):
        s = support.spec()
        s["kinds"]["like.fix"] = {"touches": ["likes"]}
        self.assertProblem("kind like.fix: touches likes of class M; optimistic kinds only touch C and P tables", s)
        s["kinds"]["like.fix"]["optimistic"] = False
        self.assertEqual(problems(s), [])

    def test_tombstone_without_version_needs_random_id(self):
        sc = support.schema()
        sc["core"]["events"]["columns"][0] = col("id", "bigint", True)
        s = support.spec()
        s["tables"]["events"] = {"class": "A", "delete": "tombstone", "insert_only": True}
        self.assertProblem("events: tombstone on an unversioned table needs a random uuid primary key", s, sc)

    def test_undeclared_trigger(self):
        sc = support.schema()
        sc["core"]["items"]["triggers"].append({"name": "items_touch", "function": "f", "enabled": "O"})
        self.assertProblem("items: trigger items_touch is not generated and not listed as derived", sc=sc)

    def test_k_needs_register(self):
        s = support.spec()
        s["tables"]["reads"]["register"] = "nope"
        self.assertProblem("reads: K table needs a register column", s)

    def test_rest_group_takes_the_remaining_columns(self):
        m = support.build()
        self.assertEqual(m.tables["marks"].groups["row"]["cols"], ["item_id", "note", "user_id"])

    def test_publication_lists(self):
        lists = {(p, t): cols for p, t, cols in support.build().publication_lists()}
        self.assertEqual(lists[("scd_core_pub", "items")], ["id", "title", "urn"])
        self.assertEqual(lists[("scd_api_pub", "likes")], ["item_id", "user_id", "wanted", "wanted_v"])
        self.assertEqual(lists[("scd_core_pub", "likes")], ["item_id", "progress", "result_v", "synced_at", "user_id", "wanted", "wanted_v"])
        self.assertNotIn(("scd_core_pub", "jobs_q"), lists)
        self.assertNotIn(("scd_core_pub", "reads"), lists)
        self.assertIn(("scd_api_pub", "reads"), lists)

    def test_partial_unique_declared(self):
        sc = support.schema()
        sc["core"]["marks"]["unique"].append(uniq("marks_note_uq", ["note"], "(note IS NOT NULL)"))
        self.assertProblem("marks: unique index marks_note_uq must be declared", sc=sc)


if __name__ == "__main__":
    unittest.main()
