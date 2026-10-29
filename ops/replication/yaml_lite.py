import re

INT = re.compile(r"-?\d+$")


class YamlError(ValueError):
    pass


def scalar(text):
    if text in ("true", "false"):
        return text == "true"
    if text in ("null", "~", ""):
        return None
    if INT.match(text):
        return int(text)
    return text


def strip_comment(line):
    quote = None
    for i, c in enumerate(line):
        if quote:
            if c == quote:
                quote = None
        elif c in "\"'":
            quote = c
        elif c == "#" and (i == 0 or line[i - 1] in " \t"):
            return line[:i].rstrip()
    return line.rstrip()


class Flow:
    def __init__(self, text, lineno):
        self.text = text
        self.pos = 0
        self.lineno = lineno

    def fail(self, msg):
        raise YamlError("line %d: %s in %r" % (self.lineno, msg, self.text))

    def skip(self):
        while self.pos < len(self.text) and self.text[self.pos] in " \t":
            self.pos += 1

    def peek(self):
        self.skip()
        return self.text[self.pos] if self.pos < len(self.text) else ""

    def expect(self, c):
        if self.peek() != c:
            self.fail("expected %r" % c)
        self.pos += 1

    def value(self):
        c = self.peek()
        if c == "{":
            return self.mapping()
        if c == "[":
            return self.sequence()
        if c in "\"'":
            return self.quoted()
        start = self.pos
        while self.pos < len(self.text) and self.text[self.pos] not in ",]}":
            self.pos += 1
        return scalar(self.text[start : self.pos].strip())

    def key(self):
        if self.peek() in "\"'":
            return self.quoted()
        start = self.pos
        while self.pos < len(self.text) and self.text[self.pos] != ":":
            if self.text[self.pos] in ",]}":
                self.fail("missing ':'")
            self.pos += 1
        return self.text[start : self.pos].strip()

    def quoted(self):
        q = self.text[self.pos]
        self.pos += 1
        out = []
        while self.pos < len(self.text):
            c = self.text[self.pos]
            if q == '"' and c == "\\" and self.pos + 1 < len(self.text):
                out.append(self.text[self.pos + 1])
                self.pos += 2
                continue
            if c == q:
                if q == "'" and self.text[self.pos + 1 : self.pos + 2] == "'":
                    out.append("'")
                    self.pos += 2
                    continue
                self.pos += 1
                return "".join(out)
            out.append(c)
            self.pos += 1
        self.fail("unterminated string")

    def mapping(self):
        self.expect("{")
        out = {}
        if self.peek() == "}":
            self.pos += 1
            return out
        while True:
            k = self.key()
            self.expect(":")
            if k in out:
                self.fail("duplicate key %r" % k)
            out[k] = self.value()
            c = self.peek()
            self.pos += 1
            if c == "}":
                return out
            if c != ",":
                self.fail("expected ',' or '}'")

    def sequence(self):
        self.expect("[")
        out = []
        if self.peek() == "]":
            self.pos += 1
            return out
        while True:
            out.append(self.value())
            c = self.peek()
            self.pos += 1
            if c == "]":
                return out
            if c != ",":
                self.fail("expected ',' or ']'")

    def whole(self):
        v = self.value()
        if self.peek():
            self.fail("trailing text")
        return v


def inline(text, lineno):
    if text[:1] in "{[\"'":
        return Flow(text, lineno).whole()
    return scalar(text)


def split_key(text, lineno):
    if text[:1] in "\"'":
        f = Flow(text, lineno)
        k = f.quoted()
        rest = text[f.pos :]
    else:
        i = text.find(":")
        if i < 0:
            raise YamlError("line %d: expected 'key: value'" % lineno)
        k, rest = text[:i].strip(), text[i:]
    if not rest.startswith(":") or (len(rest) > 1 and rest[1] not in " \t"):
        raise YamlError("line %d: expected ': ' after key" % lineno)
    return k, rest[1:].strip()


def loads(text):
    lines = []
    for n, raw in enumerate(text.splitlines(), 1):
        if "\t" in raw[: len(raw) - len(raw.lstrip())]:
            raise YamlError("line %d: tab indentation" % n)
        line = strip_comment(raw)
        if line.strip():
            lines.append((n, len(line) - len(line.lstrip()), line.strip()))
    value, pos = block(lines, 0, 0)
    if pos != len(lines):
        raise YamlError("line %d: unexpected indentation" % lines[pos][0])
    return value


def block(lines, pos, indent):
    if pos >= len(lines):
        return None, pos
    if lines[pos][2].startswith("- ") or lines[pos][2] == "-":
        return block_list(lines, pos, lines[pos][1])
    return block_map(lines, pos, lines[pos][1])


def block_list(lines, pos, indent):
    out = []
    while pos < len(lines) and lines[pos][1] == indent and lines[pos][2].startswith("-"):
        n, _, text = lines[pos]
        item = text[1:].strip()
        if not item:
            raise YamlError("line %d: empty list item" % n)
        out.append(inline(item, n))
        pos += 1
    return out, pos


def block_map(lines, pos, indent):
    out = {}
    while pos < len(lines) and lines[pos][1] == indent:
        n, _, text = lines[pos]
        if text.startswith("-"):
            raise YamlError("line %d: list item inside a mapping" % n)
        k, rest = split_key(text, n)
        if k in out:
            raise YamlError("line %d: duplicate key %r" % (n, k))
        pos += 1
        if rest:
            out[k] = inline(rest, n)
        elif pos < len(lines) and lines[pos][1] > indent:
            out[k], pos = block(lines, pos, lines[pos][1])
        else:
            out[k] = None
    if pos < len(lines) and lines[pos][1] > indent:
        raise YamlError("line %d: unexpected indentation" % lines[pos][0])
    return out, pos


def load(path):
    with open(path, encoding="utf-8") as f:
        return loads(f.read())
