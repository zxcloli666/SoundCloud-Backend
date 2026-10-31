import unittest

import support
import model
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

    def test_kinds_touch_c_p_and_m_only(self):
        s = support.spec()
        s["kinds"]["like.fix"] = {"touches": ["likes"]}
        m = support.build(s)
        self.assertEqual(m.problems(), [])
        self.assertTrue(m.held(m.tables["likes"]))
        s["kinds"]["like.fix"]["optimistic"] = False
        m = support.build(s)
        self.assertFalse(m.held(m.tables["likes"]))
        s["kinds"]["mark.fix"] = {"touches": ["marks"]}
        self.assertProblem("kind mark.fix: touches marks of class A; kinds only touch C, P and M tables", s)

    def test_tombstone_without_version_needs_random_id(self):
        sc = support.schema()
        sc["core"]["events"]["columns"][0] = col("id", "bigint", True)
        s = support.spec()
        s["tables"]["events"] = {"class": "A", "delete": "tombstone", "insert_only": True}
        self.assertProblem("events: tombstone on an unversioned table needs a random uuid primary key", s, sc)

    def test_delete_wins_only_for_random_uuid_keys(self):
        m = model.load()
        wins = sorted(t.name for t in m.installed() if t.delete_wins())
        self.assertEqual(wins, ["featured_items", "link_requests", "listening_history", "login_requests", "sessions"])
        for name in ("disliked_tracks", "discover_promoted", "subscriptions", "user_superlikes", "sc_egress_health"):
            self.assertFalse(next(t for t in m.installed() if t.name == name).delete_wins(), name)

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
        self.assertEqual(lists[("scd_core_pub", "likes")], ["item_id", "progress", "synced_at", "user_id", "wanted", "wanted_v"])
        self.assertNotIn(("scd_core_pub", "jobs_q"), lists)
        self.assertNotIn(("scd_core_pub", "reads"), lists)
        self.assertIn(("scd_api_pub", "reads"), lists)

    def test_core_group_has_no_version(self):
        s = support.spec()
        s["tables"]["likes"]["groups"]["result"]["version"] = "result_v"
        self.assertProblem("likes.result: a group with one writer (core) has no version; drop result_v", s)

    def test_api_group_needs_a_version(self):
        s = support.spec()
        del s["tables"]["likes"]["groups"]["intent"]["version"]
        self.assertProblem("likes.intent: group needs a version column", s)

    def test_least_defaults_to_created_at_in_a_versioned_group(self):
        sc = support.schema()
        sc["core"]["marks"]["columns"].append(col("created_at", "timestamptz", True, "now()"))
        self.assertEqual(support.build(sc=sc).tables["marks"].least, ["created_at"])
        self.assertEqual(support.build().tables["marks"].least, [])

    def test_least_must_be_in_a_versioned_group(self):
        s = support.spec()
        s["tables"]["likes"]["least"] = ["synced_at"]
        self.assertProblem("likes: least column synced_at must be in a versioned group", s)
        s["tables"]["likes"]["least"] = []
        self.assertEqual(problems(s), [])

    def test_held_c_unique_needs_key_or_reassert(self):
        s = support.spec()
        del s["tables"]["items"]["unique"]
        self.assertProblem("items: unique index items_urn_uq of a C table an optimistic kind touches needs key", s)
        s["tables"]["items"]["reassert"] = "always"
        self.assertEqual(problems(s), [])
        s["tables"]["items"]["unique"] = {"items_urn_uq": "key"}
        self.assertProblem("items: reassert: always already covers every unique", s)
        s["tables"]["items"]["unique"] = {"items_urn_uq": "id"}
        del s["tables"]["items"]["reassert"]
        self.assertProblem("items: unique items_urn_uq on a C table must be key", s)
        s = support.spec()
        s["kinds"] = {}
        del s["tables"]["items"]["unique"]
        self.assertEqual(problems(s), [])
        s["tables"]["items"]["reassert"] = "always"
        self.assertProblem("items: reassert: always on a table no optimistic kind touches", s)
        s["tables"]["items"]["reassert"] = "sometimes"
        self.assertProblem("items: reassert must be always", s)
        s = support.spec()
        s["tables"]["likes"]["reassert"] = "always"
        self.assertProblem("likes: reassert only on C tables", s)

    def test_insert_only_least_is_a_non_key_column(self):
        s = support.spec()
        s["tables"]["events"]["least"] = ["body"]
        self.assertEqual(problems(s), [])
        s["tables"]["events"]["least"] = ["id"]
        self.assertProblem("events: least column id must be a non-key column", s)
        s["tables"]["events"]["least"] = ["nope"]
        self.assertProblem("events: least column nope must be a non-key column", s)

    def test_derived_trigger_must_be_always_gated_and_pinned(self):
        s = support.spec()
        s["tables"]["items"]["derived"] = {"items_touch": {}}
        sc = support.schema()
        sc["core"]["items"]["triggers"].append({"name": "items_touch", "function": "touch", "enabled": "O", "gated": False, "pinned": False})
        found = problems(s, sc)
        self.assertIn("items: derived trigger items_touch must be ENABLE ALWAYS (R8)", found)
        self.assertIn("items: derived trigger items_touch needs WHEN (public.scd_derived()) (R8)", found)
        self.assertIn("items: derived trigger function touch needs SET search_path (lab D3)", found)
        sc["core"]["items"]["triggers"][-1].update(enabled="A", gated=True, pinned=True)
        self.assertEqual(problems(s, sc), [])

    def test_derived_writes_must_be_declared(self):
        s = support.spec()
        s["tables"]["items"]["derived"] = {"items_touch": {}}
        sc = support.schema()
        source = "BEGIN UPDATE public.items AS i SET title = upper(NEW.title), last_read_at = coalesce(i.last_read_at, now()) WHERE i.id = NEW.id; RETURN NULL; END"
        sc["core"]["items"]["triggers"].append({"name": "items_touch", "function": "touch", "enabled": "A", "gated": True, "pinned": True,
                                                "before": False, "source": source})
        found = problems(s, sc)
        self.assertIn("items: derived trigger items_touch writes items.title; register it under writes (a core column) or vetoes", found)
        self.assertIn("items: derived trigger items_touch writes items.last_read_at; register it under writes (a core column) or vetoes", found)
        s["tables"]["items"]["derived"] = {"items_touch": {"writes": ["items.title", "items.last_read_at", "items.gone"]}}
        found = problems(s, sc)
        self.assertIn("items: derived trigger items_touch does not write items.gone; drop it from the declaration", found)
        self.assertIn("items.last_read_at: a core column must be a replicated non-key column", found)
        s["tables"]["items"]["derived"] = {"items_touch": {"writes": ["items.title"], "vetoes": ["items.last_read_at"]}}
        self.assertEqual(problems(s, sc), [])
        self.assertEqual(support.build(s, sc).tables["items"].core_cols, ["title"])

    def test_derived_rows_and_vetoes(self):
        s = support.spec()
        s["tables"]["items"]["derived"] = {"items_touch": {}}
        s["kinds"] = {"item.edit": {"touches": ["items", "item_tags"]}}
        sc = support.schema()
        source = "BEGIN IF EXISTS (SELECT 1) THEN NEW.title := NULL; RETURN NULL; END IF; DELETE FROM item_tags WHERE item_id = NEW.id; RETURN NEW; END"
        sc["core"]["items"]["triggers"].append({"name": "items_touch", "function": "touch", "enabled": "A", "gated": True, "pinned": True,
                                                "before": True, "source": source})
        found = problems(s, sc)
        self.assertIn("items: derived trigger items_touch writes rows of item_tags; list it under rows or vetoes", found)
        self.assertIn("items: derived trigger items_touch writes rows of items; list it under rows or vetoes", found)
        s["tables"]["items"]["derived"] = {"items_touch": {"vetoes": ["items", "items.title"], "rows": ["item_tags"]}}
        self.assertEqual([p for p in problems(s, sc) if "items_touch" in p],
                         ["kind item.edit: touches items and item_tags, whose rows the derived trigger items_touch writes on core only"])
        self.assertEqual(support.build(s, sc).tables["items"].core_cols, [])

    def test_fk_parents_and_orphan_fks(self):
        sc = support.schema()
        sc["core"]["marks"]["fks"].append(fk("marks_like_fkey", ["user_id", "item_id"], "likes", ["user_id", "item_id"], "c"))
        m = support.build(sc=sc)
        self.assertEqual([t.name for t in m.fk_parents()], ["likes"])
        self.assertEqual([f["name"] for f in m.orphan_fks(m.tables["marks"])], ["marks_like_fkey"])
        self.assertEqual(m.orphan_fks(m.tables["item_tags"]), [])

    def test_partial_unique_declared(self):
        sc = support.schema()
        sc["core"]["marks"]["unique"].append(uniq("marks_note_uq", ["note"], "(note IS NOT NULL)"))
        self.assertProblem("marks: unique index marks_note_uq must be declared", sc=sc)


if __name__ == "__main__":
    unittest.main()
