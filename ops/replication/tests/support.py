import copy
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPL = os.path.dirname(HERE)
sys.path.insert(0, REPL)

import model


def col(name, type_="text", notnull=False, default=None, generated=False):
    return {"name": name, "type": type_, "notnull": notnull, "default": default, "generated": generated}


def pk(cols, name="pkey"):
    return {"name": name, "primary": True, "cols": cols, "expr": False, "keys": cols, "where": None}


def uniq(name, cols, where=None):
    return {"name": name, "primary": False, "cols": cols, "expr": False, "keys": cols, "where": where}


def fk(name, cols, ref_table, ref_cols, on_delete="a"):
    return {"name": name, "cols": cols, "ref_table": ref_table, "ref_cols": ref_cols, "on_delete": on_delete}


def table(columns, unique, fks=(), triggers=()):
    return {"columns": columns, "unique": unique, "fks": list(fks), "triggers": [{"name": t, "function": t, "enabled": "O"} for t in triggers]}


SCHEMA = {
    "core": {
        "items": table(
            [col("id", "uuid", True, "gen_random_uuid()"), col("urn", "text", True), col("title"), col("last_read_at", "timestamptz")],
            [pk(["id"]), uniq("items_urn_uq", ["urn"])],
        ),
        "item_tags": table(
            [col("item_id", "uuid", True), col("tag", "text", True)],
            [pk(["item_id", "tag"])],
            [fk("item_tags_item_fkey", ["item_id"], "items", ["id"], "c")],
        ),
        "jobs_q": table([col("id", "bigint", True), col("item_id", "uuid")], [pk(["id"])], [fk("jobs_q_item_fkey", ["item_id"], "items", ["id"], "n")]),
        "likes": table(
            [col("user_id", "text", True), col("item_id", "text", True), col("wanted", "boolean", True, "true"),
             col("progress", "boolean", True, "false"), col("synced_at", "timestamptz")],
            [pk(["user_id", "item_id"])],
        ),
        "marks": table(
            [col("id", "uuid", True, "gen_random_uuid()"), col("user_id", "text", True), col("item_id", "text", True), col("note")],
            [pk(["id"]), uniq("marks_user_item_uq", ["user_id", "item_id"])],
        ),
        "events": table([col("id", "uuid", True, "gen_random_uuid()"), col("body")], [pk(["id"])]),
        "reads": table([col("urn", "text", True), col("at", "timestamptz", True)], [pk(["urn"])]),
    },
    "ops": {},
}

SPEC = {
    "version": 1,
    "publications": {
        "scd_core_pub": {"node": "core", "publish": "insert, update, delete"},
        "scd_api_pub": {"node": "api", "publish": "insert, update, delete"},
    },
    "kinds": {"item.edit": {"touches": ["items"]}},
    "tables": {
        "items": {"class": "C", "local": ["last_read_at"]},
        "item_tags": {"class": "C"},
        "jobs_q": {"class": "P"},
        "likes": {
            "class": "M",
            "delete": "receipt",
            "groups": {
                "intent": {"owner": "api", "version": "wanted_v", "cols": ["wanted"]},
                "result": {"owner": "core", "version": "result_v", "cols": ["progress", "synced_at"]},
            },
        },
        "marks": {
            "class": "A",
            "delete": "tombstone",
            "id": {"ns": "marks", "from": ["user_id", "item_id"]},
            "unique": {"marks_user_item_uq": "id"},
            "groups": {"row": {"version": "row_v", "cols": "rest"}},
        },
        "events": {"class": "A", "delete": "none", "insert_only": True},
        "reads": {"class": "K", "register": "at"},
    },
}


def spec():
    return copy.deepcopy(SPEC)


def schema():
    return copy.deepcopy(SCHEMA)


def build(s=None, sc=None):
    return model.Model(s if s is not None else spec(), sc if sc is not None else schema())
