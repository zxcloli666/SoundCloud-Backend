from __future__ import annotations

import math
from collections.abc import Mapping

TYPES: Mapping[str, type | tuple[type, ...]] = {
    "object": dict,
    "array": list,
    "string": str,
    "integer": int,
    "number": (int, float),
    "boolean": bool,
}


def validate(value: object, schema: Mapping[str, object], where: str = "$") -> list[str]:
    if "anyOf" in schema:
        return any_of(value, schema, where)
    problems = type_problems(value, schema, where)
    if problems:
        return problems
    problems += value_problems(value, schema, where)
    if isinstance(value, dict):
        problems += object_problems(value, schema, where)
    elif isinstance(value, list):
        problems += array_problems(value, schema, where)
    return problems


def any_of(value: object, schema: Mapping[str, object], where: str) -> list[str]:
    options = schema["anyOf"]
    if not isinstance(options, list):
        return [f"{where}: anyOf must list schemas"]
    for option in options:
        if isinstance(option, Mapping) and not validate(value, option, where):
            return []
    return [f"{where}: matches none of {len(options)} alternatives"]


def type_problems(value: object, schema: Mapping[str, object], where: str) -> list[str]:
    expected = schema.get("type")
    if expected is None:
        return []
    names = expected if isinstance(expected, list) else [expected]
    if any(isinstance(name, str) and has_type(value, name) for name in names):
        return []
    return [f"{where}: expected {'|'.join(map(str, names))}, got {json_type(value)}"]


def has_type(value: object, name: str) -> bool:
    if name == "null":
        return value is None
    if name in ("integer", "number") and isinstance(value, bool):
        return False
    if name == "number" and isinstance(value, float) and not math.isfinite(value):
        return False
    expected = TYPES.get(name)
    return expected is not None and isinstance(value, expected)


def json_type(value: object) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    for name, python in TYPES.items():
        if isinstance(value, python):
            return name
    return type(value).__name__


def value_problems(value: object, schema: Mapping[str, object], where: str) -> list[str]:
    problems: list[str] = []
    if "const" in schema and not same(value, schema["const"]):
        problems.append(f"{where}: expected {schema['const']!r}")
    enum = schema.get("enum")
    if isinstance(enum, list) and not any(same(value, option) for option in enum):
        problems.append(f"{where}: {value!r} is not one of {enum}")
    if isinstance(value, int | float) and not isinstance(value, bool):
        minimum, maximum = schema.get("minimum"), schema.get("maximum")
        if isinstance(minimum, int | float) and value < minimum:
            problems.append(f"{where}: {value} is below {minimum}")
        if isinstance(maximum, int | float) and value > maximum:
            problems.append(f"{where}: {value} is above {maximum}")
    if isinstance(value, str):
        problems += bounds(len(value), schema, "minLength", "maxLength", where)
    return problems


def same(value: object, expected: object) -> bool:
    return type(value) is type(expected) and value == expected


def object_problems(
    value: dict[object, object], schema: Mapping[str, object], where: str
) -> list[str]:
    problems: list[str] = []
    properties = schema.get("properties")
    known: Mapping[str, object] = properties if isinstance(properties, Mapping) else {}
    required = schema.get("required")
    for key in required if isinstance(required, list) else []:
        if key not in value:
            problems.append(f"{where}: missing {key!r}")
    for key, item in value.items():
        rule = known.get(key) if isinstance(key, str) else None
        if isinstance(rule, Mapping):
            problems += validate(item, rule, f"{where}.{key}")
        elif schema.get("additionalProperties") is False:
            problems.append(f"{where}: unexpected {key!r}")
    return problems


def array_problems(value: list[object], schema: Mapping[str, object], where: str) -> list[str]:
    problems = bounds(len(value), schema, "minItems", "maxItems", where)
    items = schema.get("items")
    if isinstance(items, Mapping):
        for index, item in enumerate(value):
            problems += validate(item, items, f"{where}[{index}]")
    return problems


def bounds(size: int, schema: Mapping[str, object], low: str, high: str, where: str) -> list[str]:
    least, most = schema.get(low), schema.get(high)
    if isinstance(least, int) and size < least:
        return [f"{where}: size {size} is below {low}={least}"]
    if isinstance(most, int) and size > most:
        return [f"{where}: size {size} is above {high}={most}"]
    return []
