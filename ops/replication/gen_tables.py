from sqlbits import q, table


def render(model):
    out = []
    for t in model.installed():
        if t.planned and not t.exists and t.spec.get("columns"):
            out.append(create_table(t))
    for t in model.installed():
        for col, decl in sorted(t.added.items()):
            out.append("ALTER TABLE %s ADD COLUMN IF NOT EXISTS %s %s;\n" % (table(t.name), q(col), decl))
    return "\n".join(out)


def create_table(t):
    lines = []
    for col, decl in t.spec["columns"].items():
        lines.append("    %s %s" % (q(col), column_decl(col, str(decl), t)))
    lines.append("    PRIMARY KEY (%s)" % ", ".join(q(c) for c in t.pk))
    return "CREATE TABLE IF NOT EXISTS %s (\n%s\n);\n" % (table(t.name), ",\n".join(lines))


def column_decl(col, decl, t):
    if col in t.pk and "NOT NULL" not in decl.upper():
        return decl + " NOT NULL"
    return decl
