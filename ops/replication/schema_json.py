import json
import sys


def compact(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=True)


def render(doc):
    lines = ["{"]
    dbs = sorted(doc)
    for di, db in enumerate(dbs):
        lines.append(' "%s": {' % db)
        tables = sorted(doc[db])
        for ti, name in enumerate(tables):
            table = doc[db][name]
            lines.append('  "%s": {' % name)
            keys = sorted(table)
            for ki, key in enumerate(keys):
                items = table[key]
                lines.append('   "%s": [' % key)
                for ii, item in enumerate(items):
                    lines.append("    " + compact(item) + ("," if ii < len(items) - 1 else ""))
                lines.append("   ]" + ("," if ki < len(keys) - 1 else ""))
            lines.append("  }" + ("," if ti < len(tables) - 1 else ""))
        lines.append(" }" + ("," if di < len(dbs) - 1 else ""))
    lines.append("}")
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    sys.stdout.write(render(json.load(sys.stdin)))
