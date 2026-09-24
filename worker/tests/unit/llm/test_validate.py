from __future__ import annotations

import pytest

from worker.llm.validate import validate

SCHEMA: dict[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": ["id", "share", "names", "flag"],
    "properties": {
        "id": {"type": ["integer", "null"], "minimum": 0},
        "share": {"type": "number", "minimum": 0, "maximum": 1},
        "names": {"type": "array", "items": {"type": "string", "minLength": 1}, "maxItems": 2},
        "flag": {"type": "boolean"},
        "kind": {"enum": ["a", "b"]},
        "version": {"const": 2},
        "album": {
            "anyOf": [
                {
                    "type": "object",
                    "required": ["title"],
                    "properties": {"title": {"type": "string"}},
                },
                {"type": "null"},
            ]
        },
    },
}
VALID: dict[str, object] = {"id": 3, "share": 0.5, "names": ["x"], "flag": False}


def with_(**changes: object) -> dict[str, object]:
    return {**VALID, **changes}


def test_a_valid_object_has_no_problems() -> None:
    assert validate(VALID, SCHEMA) == []
    assert validate(with_(id=None, share=1, kind="b", version=2, album=None), SCHEMA) == []
    assert validate(with_(album={"title": "GNX"}), SCHEMA) == []


@pytest.mark.parametrize(
    ("changes", "problem"),
    [
        ({"id": True}, "$.id: expected integer|null, got boolean"),
        ({"share": False}, "$.share: expected number, got boolean"),
        ({"id": 1.0}, "$.id: expected integer|null, got number"),
        ({"share": float("nan")}, "$.share: expected number, got number"),
        ({"flag": 0}, "$.flag: expected boolean, got integer"),
        ({"id": -1}, "$.id: -1 is below 0"),
        ({"share": 1.5}, "$.share: 1.5 is above 1"),
        ({"names": ["x", "y", "z"]}, "$.names: size 3 is above maxItems=2"),
        ({"names": [""]}, "$.names[0]: size 0 is below minLength=1"),
        ({"names": [1]}, "$.names[0]: expected string, got integer"),
        ({"kind": "c"}, "$.kind: 'c' is not one of ['a', 'b']"),
        ({"version": 2.0}, "$.version: expected 2"),
        ({"version": True}, "$.version: expected 2"),
        ({"extra": 1}, "$: unexpected 'extra'"),
        ({"album": {"name": "x"}}, "$.album: matches none of 2 alternatives"),
    ],
)
def test_each_violation_is_reported(changes: dict[str, object], problem: str) -> None:
    assert validate(with_(**changes), SCHEMA) == [problem]


def test_missing_required_keys_are_reported() -> None:
    assert validate({"id": 1, "share": 0, "names": []}, SCHEMA) == ["$: missing 'flag'"]


def test_a_non_object_reply_fails_the_type_check() -> None:
    assert validate(["id"], SCHEMA) == ["$: expected object, got array"]
    assert validate(None, SCHEMA) == ["$: expected object, got null"]
