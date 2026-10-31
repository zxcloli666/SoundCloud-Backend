import json
import os

import derived
import yaml_lite

HERE = os.path.dirname(os.path.abspath(__file__))
CLASSES = ("C", "P", "M", "A", "K", "L", "X")
REPLICATED = ("C", "M", "A", "K", "X")
OWNERS = ("api", "core", "both")
DELETES = {"M": ("receipt",), "A": ("none", "tombstone", "core")}
TABLE_KEYS = {
    "class", "db", "note", "planned", "prod_only", "pk", "columns", "planned_cols", "groups", "local", "private",
    "delete", "insert_only", "register", "id", "unique", "publish", "hold", "derived", "unpublished_fks", "least",
}
FK_ACTIONS = {"a": "no action", "r": "restrict", "c": "cascade", "n": "set null", "d": "set default"}


class Table:
    def __init__(self, name, spec, schema):
        self.name = name
        self.spec = spec
        self.cls = spec.get("class")
        self.db = spec.get("db", "core")
        self.planned = spec.get("planned")
        self.prod_only = bool(spec.get("prod_only"))
        self.schema = schema
        self.exists = schema is not None
        self.local = list(spec.get("local") or [])
        self.private = list(spec.get("private") or [])
        self.delete = spec.get("delete", "none" if self.cls == "A" else None)
        self.insert_only = bool(spec.get("insert_only"))
        self.register = spec.get("register")
        self.id = spec.get("id")
        self.unique_decl = dict(spec.get("unique") or {})
        self.hold = spec.get("hold")
        self.derived = dict(spec.get("derived") or {})
        self.core_cols = []
        self.unpublished_fks = list(spec.get("unpublished_fks") or [])
        self.publish_to = list(spec.get("publish") or [])
        self.types = {}
        self.notnull = {}
        self.defaults = {}
        self.generated = set()
        self.uniques = []
        self.fks = []
        self.triggers = []
        self.trigger_info = {}
        self.pk = list(spec.get("pk") or [])
        if schema:
            for c in schema["columns"]:
                self.types[c["name"]] = c["type"]
                self.notnull[c["name"]] = c["notnull"]
                self.defaults[c["name"]] = c["default"]
                if c["generated"]:
                    self.generated.add(c["name"])
            for u in schema["unique"]:
                if u["primary"]:
                    self.pk = list(u["cols"])
                else:
                    self.uniques.append(u)
            self.fks = schema["fks"]
            self.triggers = [t["name"] for t in schema["triggers"]]
            self.trigger_info = {t["name"]: t for t in schema["triggers"]}
        elif spec.get("columns"):
            for col, decl in spec["columns"].items():
                self.types[col], self.notnull[col], self.defaults[col] = parse_decl(decl)
        self.added = {}
        for col, decl in (spec.get("planned_cols") or {}).items():
            if col not in self.types:
                self.added[col] = decl
                self.types[col], self.notnull[col], self.defaults[col] = parse_decl(decl)
        self.groups = {}
        for gname, g in (spec.get("groups") or {}).items():
            owner = g.get("owner", "both" if self.cls == "A" else None)
            self.groups[gname] = {"owner": owner, "version": g.get("version"), "cols": g.get("cols")}
            v = g.get("version")
            if v and v not in self.types:
                self.added[v] = "bigint NOT NULL DEFAULT 0"
                self.types[v], self.notnull[v], self.defaults[v] = "bigint", True, "0"
        self.versions = [g["version"] for g in self.groups.values() if g["version"]]
        self.columns = sorted(self.types)
        self.replicated = [c for c in self.columns if c not in self.local and c not in self.private and c not in self.generated]
        for g in self.groups.values():
            if g["cols"] == "rest":
                taken = {c for o in self.groups.values() if isinstance(o["cols"], list) for c in o["cols"]}
                g["cols"] = [c for c in self.replicated if c not in self.pk and c not in self.versions and c not in taken]
        if "least" in spec:
            self.least = list(spec.get("least") or [])
        else:
            self.least = [c for c in ("created_at",) if any(g["version"] and c in (g["cols"] or []) for g in self.groups.values())]

    @property
    def replicated_class(self):
        return self.cls in REPLICATED

    @property
    def versioned(self):
        return bool(self.groups)

    def group_cols(self, owners=OWNERS):
        return sorted({c for g in self.groups.values() if g["owner"] in owners for c in g["cols"] + ([g["version"]] if g["version"] else [])})

    def versioned_groups(self):
        return [(n, g) for n, g in sorted(self.groups.items()) if g["version"]]

    def group_of(self, col):
        return next((g for g in self.groups.values() if col in (g["cols"] or [])), None)

    def api_groups(self):
        return [(n, g) for n, g in sorted(self.groups.items()) if g["owner"] in ("api", "both")]

    def core_groups(self):
        return [(n, g) for n, g in sorted(self.groups.items()) if g["owner"] == "core"]

    def unique_index(self, name):
        return next((u for u in self.uniques if u["name"] == name), None)

    def random_pk(self):
        return len(self.pk) == 1 and (self.defaults.get(self.pk[0]) or "").startswith("gen_random_uuid()")

    def delete_wins(self):
        return self.delete == "tombstone" and not self.id and len(self.pk) == 1 and self.types.get(self.pk[0]) == "uuid"


def parse_decl(decl):
    text = str(decl)
    upper = text.upper()
    default = None
    if " DEFAULT " in upper:
        i = upper.index(" DEFAULT ")
        default = text[i + 9 :].strip()
        text = text[:i]
        upper = upper[:i]
    notnull = "NOT NULL" in upper
    typ = text[: upper.index(" NOT NULL")] if notnull else text
    return typ.strip(), notnull, default


class Model:
    def __init__(self, spec, schema):
        self.spec = spec
        self.schema = schema
        self.publications = spec.get("publications") or {}
        self.kinds = spec.get("kinds") or {}
        self.core_only_code = spec.get("core_only_code") or {}
        self.tables = {}
        for name, tspec in sorted((spec.get("tables") or {}).items()):
            db = (tspec or {}).get("db", "core")
            self.tables[name] = Table(name, tspec or {}, schema.get(db, {}).get(name))
        for name, cols in derived.core_columns(self).items():
            if name in self.tables:
                self.tables[name].core_cols = cols

    def by_class(self, *classes):
        return [t for t in self.tables.values() if t.cls in classes]

    def installed(self):
        return [t for t in self.tables.values() if t.db == "core" and not t.prod_only]

    def publication_lists(self):
        lists = []
        for t in self.installed():
            if t.cls in ("C", "M", "A") and not t.hold:
                lists.append(("scd_core_pub", t.name, t.replicated))
            if t.cls == "M":
                lists.append(("scd_api_pub", t.name, sorted(set(t.pk) | set(t.group_cols(("api", "both"))))))
            if t.cls in ("A", "K"):
                lists.append(("scd_api_pub", t.name, t.replicated))
            if t.cls == "X":
                for pub in t.publish_to:
                    lists.append((pub, t.name, t.replicated))
        return sorted(lists)

    def fk_parents(self):
        return [t for t in self.installed() if t.cls in ("M", "A") and t.exists and self.children(t.name)]

    def orphan_fks(self, t):
        if t.cls not in ("M", "A"):
            return []
        out = []
        for fk in t.fks:
            parent = self.tables.get(fk["ref_table"])
            if parent and parent.replicated_class and parent.name != t.name:
                out.append(fk)
        return out

    def held(self, t):
        return t.cls == "M" and any(k.get("optimistic", True) and t.name in (k.get("touches") or []) for k in self.kinds.values())

    def children(self, parent):
        out = []
        for t in self.installed():
            for fk in t.fks:
                if fk["ref_table"] == parent:
                    out.append((t, fk))
        return sorted(out, key=lambda x: (x[0].name, x[1]["name"]))

    def problems(self):
        out = []
        for name, t in self.tables.items():
            out += table_problems(self, t)
        for db, tables in sorted(self.schema.items()):
            for name in sorted(tables):
                if name not in self.tables:
                    out.append("%s: table in the %s schema but not in ownership.yaml" % (name, db))
                elif self.tables[name].db != db:
                    out.append("%s: ownership.yaml says db %s, schema has it in %s" % (name, self.tables[name].db, db))
        for kind, k in sorted(self.kinds.items()):
            for tname in k.get("touches") or []:
                t = self.tables.get(tname)
                if not t:
                    out.append("kind %s: touches unknown table %s" % (kind, tname))
                elif t.cls not in ("C", "P", "M"):
                    out.append("kind %s: touches %s of class %s; kinds only touch C, P and M tables" % (kind, tname, t.cls))
        for pub, p in sorted(self.publications.items()):
            if p.get("node") not in ("core", "api"):
                out.append("publication %s: node must be core or api" % pub)
        out += derived.problems(self)
        out += cycle_problems(self)
        out += insert_problems(self)
        return out


def table_problems(model, t):
    out = []
    n = t.name
    for k in sorted(set(t.spec) - TABLE_KEYS):
        out.append("%s: unknown key %r" % (n, k))
    if t.cls not in CLASSES:
        return out + ["%s: class must be one of %s" % (n, "/".join(CLASSES))]
    if not t.exists and not t.planned and not t.prod_only:
        out.append("%s: not in the schema; mark it planned: <package> or prod_only: true" % n)
    if t.planned and t.cls not in ("L", "P") and not t.spec.get("columns"):
        out.append("%s: planned %s table needs columns" % (n, t.cls))
    if t.replicated_class and not t.pk and not t.prod_only:
        out.append("%s: replicated table without a primary key" % n)
    for c in t.local + t.private:
        if c not in t.types:
            out.append("%s: local/private column %s does not exist" % (n, c))
        if c in t.pk:
            out.append("%s: key column %s cannot be local or private" % (n, c))
    if t.private and t.cls != "C":
        out.append("%s: private columns exist only on C tables" % n)
    if t.cls in ("M", "A"):
        out += group_problems(t)
    elif t.groups:
        out.append("%s: groups only on M and A tables" % n)
    if t.spec.get("least") is not None and t.cls not in ("M", "A"):
        out.append("%s: least only on M and A tables" % n)
    if t.cls in DELETES and t.delete not in DELETES[t.cls]:
        out.append("%s: delete must be one of %s" % (n, "/".join(DELETES[t.cls])))
    if t.cls not in DELETES and t.spec.get("delete"):
        out.append("%s: delete policy only on M and A tables" % n)
    if t.cls == "A" and t.delete == "tombstone" and not t.versioned and not t.random_pk():
        out.append("%s: tombstone on an unversioned table needs a random uuid primary key" % n)
    if t.insert_only and t.cls != "A":
        out.append("%s: insert_only only on A tables" % n)
    if t.cls == "K" and t.register not in t.types:
        out.append("%s: K table needs a register column" % n)
    if t.register and t.cls != "K":
        out.append("%s: register only on K tables" % n)
    if t.hold and t.cls != "C":
        out.append("%s: hold only on C tables" % n)
    if t.publish_to and t.cls != "X":
        out.append("%s: publish only on X tables" % n)
    for pub in t.publish_to:
        if pub not in model.publications:
            out.append("%s: unknown publication %s" % (n, pub))
    out += unique_problems(t)
    out += trigger_problems(t)
    out += fk_problems(model, t)
    return out


def group_problems(t):
    out = []
    n = t.name
    if t.insert_only:
        if t.groups:
            out.append("%s: insert_only tables are not versioned" % n)
        return out
    if not t.groups:
        return ["%s: %s table needs version groups" % (n, t.cls)]
    seen = {}
    for gname, g in sorted(t.groups.items()):
        if g["owner"] not in OWNERS:
            out.append("%s.%s: owner must be api, core or both" % (n, gname))
        if g["owner"] == "core" and g["version"]:
            out.append("%s.%s: a group with one writer (core) has no version; drop %s (lab D1)" % (n, gname, g["version"]))
        if g["owner"] != "core" and not g["version"]:
            out.append("%s.%s: group needs a version column" % (n, gname))
        if not isinstance(g["cols"], list) or not g["cols"]:
            out.append("%s.%s: group needs columns" % (n, gname))
            continue
        for c in g["cols"]:
            if c not in t.types:
                out.append("%s.%s: column %s does not exist" % (n, gname, c))
            elif c in t.pk or c in t.versions or c in t.local or c in t.generated:
                out.append("%s.%s: column %s is a key, version, local or generated column" % (n, gname, c))
            if c in seen:
                out.append("%s: column %s is in groups %s and %s" % (n, c, seen[c], gname))
            seen[c] = gname
    if t.cls == "M" and not t.core_groups():
        out.append("%s: M table needs a core-owned result group" % n)
    if t.cls == "M" and not t.api_groups():
        out.append("%s: M table needs an api-owned intent group" % n)
    for c in t.replicated:
        if c not in t.pk and c not in t.versions and c not in seen:
            out.append("%s: column %s is in no group; add it to a group or to local" % (n, c))
    for c in t.least:
        g = t.group_of(c)
        if not g or not g["version"]:
            out.append("%s: least column %s must be in a versioned group" % (n, c))
    return out


def unique_problems(t):
    out = []
    n = t.name
    if t.cls in ("M", "A", "K"):
        names = {u["name"] for u in t.uniques}
        for u in t.uniques:
            if u["name"] not in t.unique_decl:
                out.append("%s: unique index %s must be declared as id (R6) or random" % (n, u["name"]))
        for name, how in sorted(t.unique_decl.items()):
            if name not in names and t.exists:
                out.append("%s: declared unique index %s does not exist" % (n, name))
            if how not in ("id", "random"):
                out.append("%s: unique %s must be id or random" % (n, name))
            u = t.unique_index(name)
            if how == "id" and u is not None:
                frm = (t.id or {}).get("from") or []
                keys = u["cols"] or []
                if u["cols"] is None:
                    keys = [c for c in frm if any(c in k for k in u["keys"])]
                if sorted(keys) != sorted(frm):
                    out.append("%s: id from %s does not match unique index %s" % (n, frm, name))
    elif t.unique_decl:
        out.append("%s: unique declarations only on M, A and K tables" % n)
    if t.id:
        if not t.id.get("ns") or not t.id.get("from"):
            out.append("%s: id needs ns and from" % n)
        for c in t.id.get("from") or []:
            if c not in t.types:
                out.append("%s: id column %s does not exist" % (n, c))
    return out


def trigger_problems(t):
    out = []
    for tr in t.triggers:
        if not tr.startswith("zz") and tr not in t.derived:
            out.append("%s: trigger %s is not generated and not listed as derived" % (t.name, tr))
    for d in t.derived:
        if t.exists and d not in t.triggers:
            out.append("%s: derived trigger %s does not exist" % (t.name, d))
    if t.derived and t.cls != "C":
        out.append("%s: derived triggers only on C tables" % t.name)
    for d in t.derived:
        info = t.trigger_info.get(d)
        if not info:
            continue
        if info.get("enabled") != "A":
            out.append("%s: derived trigger %s must be ENABLE ALWAYS (R8)" % (t.name, d))
        if not info.get("gated"):
            out.append("%s: derived trigger %s needs WHEN (public.scd_derived()) (R8)" % (t.name, d))
        if not info.get("pinned"):
            out.append("%s: derived trigger function %s needs SET search_path (lab D3)" % (t.name, info.get("function")))
    return out


def fk_problems(model, t):
    out = []
    names = set()
    for fk in t.fks:
        names.add(fk["name"])
        parent = model.tables.get(fk["ref_table"])
        if not parent:
            continue
        crossing = t.replicated_class and not parent.replicated_class
        if crossing and fk["name"] not in t.unpublished_fks:
            out.append("%s: FK %s points at %s table %s; list it in unpublished_fks" % (t.name, fk["name"], parent.cls, parent.name))
        if not crossing and fk["name"] in t.unpublished_fks:
            out.append("%s: FK %s is listed in unpublished_fks but its parent is replicated" % (t.name, fk["name"]))
    for name in t.unpublished_fks:
        if name not in names:
            out.append("%s: unpublished_fks names unknown FK %s" % (t.name, name))
    return out


def cycle_problems(model):
    edges = {}
    for t in model.installed():
        edges[t.name] = sorted({c.name for c, fk in model.children(t.name) if purge_action(c, fk) == "purge" and c.name != t.name})
    out = []
    state = {}

    def visit(n, path):
        if state.get(n) == 1:
            out.append("FK purge cycle: " + " -> ".join(path + [n]))
            return
        if state.get(n) == 2:
            return
        state[n] = 1
        for m in edges.get(n, []):
            visit(m, path + [n])
        state[n] = 2

    for n in sorted(edges):
        visit(n, [])
    return out


def insert_problems(model):
    out = []
    for pub, name, cols in model.publication_lists():
        t = model.tables[name]
        for c in t.columns:
            if c not in cols and c not in t.generated and t.notnull.get(c) and t.defaults.get(c) is None:
                out.append("%s: column %s is NOT NULL without a default and not in %s; inserts from that publication fail" % (name, c, pub))
    return out


def purge_action(child, fk):
    action = fk["on_delete"]
    if action == "c":
        return "purge"
    if action == "d":
        return "default"
    if action == "n":
        return "null"
    nullable = [c for c in fk["cols"] if not child.notnull.get(c) and c not in child.pk]
    return "null_some" if nullable else "purge"


def load(yaml_path=None, schema_path=None):
    spec = yaml_lite.load(yaml_path or os.path.join(HERE, "ownership.yaml"))
    with open(schema_path or os.path.join(HERE, "schema.json"), encoding="utf-8") as f:
        schema = json.load(f)
    return Model(spec, schema)
