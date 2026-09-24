from __future__ import annotations

import json
from collections.abc import Iterator, Mapping
from dataclasses import dataclass
from pathlib import Path
from types import MappingProxyType
from typing import Any

from jsonschema import Draft202012Validator, ValidationError, validators
from jsonschema.protocols import Validator

CONTRACT_VERSION = 2
MAX_UTF8_BYTES = "x-max-utf8-bytes"
OBJECT_PLACEHOLDER = "{object}"


class ContractError(ValueError):
    pass


@dataclass(frozen=True)
class LaneSpec:
    name: str
    stream: str
    durable: str
    filter_subject: str
    done_subject: str | None
    done_msg_id_template: str | None
    correlation_prefix: str
    correlation_separator: str
    correlation: tuple[str, ...]
    echo: Mapping[str, str]
    deadline_s: float
    ack_wait_s: float
    heartbeat_s: float
    max_deliver: int
    max_ack_pending: int
    nak_base_s: float | None
    nak_cap_s: float | None
    attempt_window_s: float | None
    quarantine_after_s: float | None
    bridge_ttl_s: float | None
    result_max_bytes: int
    result_object_template: str | None
    reasons: frozenset[str]
    worker_reasons: frozenset[str]
    reopenable: frozenset[str]
    rpc: bool
    public: bool

    def nak_delay(self, num_delivered: int) -> float:
        if self.nak_base_s is None or self.nak_cap_s is None:
            raise ContractError(f"lane {self.name} never naks")
        exponent = max(0, num_delivered - 1)
        return float(min(self.nak_base_s * 2**exponent, self.nak_cap_s))

    def is_last_delivery(self, num_delivered: int, max_deliver: int | None = None) -> bool:
        limit = self.max_deliver if max_deliver is None else max_deliver
        return limit > 0 and num_delivered >= limit

    def correlation_key(self, payload: Mapping[str, object]) -> str:
        values = []
        for field in self.correlation:
            if field not in payload or payload[field] is None:
                raise ContractError(f"lane {self.name}: correlation field {field!r} missing")
            values.append(str(payload[field]))
        return self.correlation_separator.join([self.correlation_prefix, *values])

    def echo_fields(self, payload: Mapping[str, object]) -> dict[str, object]:
        return {
            done_name: payload[name] for name, done_name in self.echo.items() if name in payload
        }

    def result_object(self, input_object: str) -> str:
        if self.result_object_template is None:
            raise ContractError(f"lane {self.name} names no result object")
        return self.result_object_template.replace(OBJECT_PLACEHOLDER, input_object)

    def done_msg_id(self, correlation: str, task_seq: int, status: str) -> str:
        if self.done_msg_id_template is None:
            raise ContractError(f"lane {self.name} publishes no done")
        return (
            self.done_msg_id_template.replace("{lane}", self.name)
            .replace("{correlation}", correlation)
            .replace("{task_seq}", str(task_seq))
            .replace("{status}", status)
        )


@dataclass(frozen=True)
class StreamSpec:
    name: str
    subjects: tuple[str, ...]
    retention: str
    max_age_s: int
    duplicate_window_s: int
    max_bytes: int
    discard: str


@dataclass(frozen=True)
class ObjectStoreSpec:
    name: str
    max_age_s: int | None


@dataclass(frozen=True)
class HeaderNames:
    msg_id: str
    reply_to: str
    deadline: str
    worker_id: str
    worker_build: str
    deliveries: str
    public_node: str


@dataclass(frozen=True)
class RpcSpec:
    reply_header: str
    deadline_header: str
    windows_s: Mapping[str, int]


@dataclass(frozen=True)
class Contract:
    version: int
    streams: Mapping[str, StreamSpec]
    object_stores: Mapping[str, ObjectStoreSpec]
    lanes: Mapping[str, LaneSpec]
    dimensions: Mapping[str, int]
    schemas: Mapping[str, Mapping[str, object]]
    validators: Mapping[str, Validator]
    reasons: Mapping[str, tuple[str, ...]]
    reason_classes: Mapping[str, tuple[str, ...]]
    headers: HeaderNames
    rpc: RpcSpec
    subjects: Mapping[str, str]
    max_message_bytes: int

    def lane(self, name: str) -> LaneSpec:
        try:
            return self.lanes[name]
        except KeyError as error:
            raise ContractError(f"unknown lane {name!r}") from error

    @property
    def public_lanes(self) -> frozenset[str]:
        return frozenset(name for name, lane in self.lanes.items() if lane.public)

    def invalid_subject(self, lane: str) -> str:
        return self.subjects["invalid"].replace("<lane>", lane)

    def status_subject(self, worker_id: str) -> str:
        return self.subjects["status"].replace("<worker_id>", worker_id)

    def health_subject(self, worker_id: str) -> str:
        return self.subjects["health"].replace("<worker_id>", worker_id)

    def validate(self, schema_name: str, payload: Any) -> list[str]:
        validator = self.validators.get(schema_name)
        if validator is None:
            raise ContractError(f"no schema {schema_name!r}")
        return sorted(
            f"{'/'.join(str(part) for part in error.absolute_path) or '$'}: {error.message}"
            for error in validator.iter_errors(payload)
        )


def load(path: Path) -> Contract:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise ContractError(f"cannot read contract {path}: {error}") from error
    return parse(raw)


def parse(raw: Mapping[str, Any]) -> Contract:
    if raw.get("version") != CONTRACT_VERSION:
        raise ContractError(f"contract version {raw.get('version')!r} != {CONTRACT_VERSION}")
    lanes = {name: parse_lane(name, spec) for name, spec in table(raw, "lanes").items()}
    streams = {name: parse_stream(name, spec) for name, spec in table(raw, "streams").items()}
    for lane in lanes.values():
        if lane.stream not in streams:
            raise ContractError(f"lane {lane.name} refers to unknown stream {lane.stream}")
    schemas = {name: dict(schema) for name, schema in table(raw, "schemas").items()}
    for schema in schemas.values():
        ContractValidator.check_schema(schema)
    compiled = {name: ContractValidator(schema) for name, schema in schemas.items()}
    rpc = table(raw, "rpc")
    return Contract(
        version=CONTRACT_VERSION,
        streams=MappingProxyType(streams),
        object_stores=MappingProxyType(
            {
                name: ObjectStoreSpec(name, spec.get("max_age_s"))
                for name, spec in table(raw, "object_stores").items()
            }
        ),
        lanes=MappingProxyType(lanes),
        dimensions=MappingProxyType(dict(table(raw, "dimensions"))),
        schemas=MappingProxyType(schemas),
        validators=MappingProxyType(compiled),
        reasons=MappingProxyType(
            {status: tuple(reasons) for status, reasons in table(raw, "reasons").items()}
        ),
        reason_classes=MappingProxyType(
            {name: tuple(reasons) for name, reasons in table(raw, "reason_classes").items()}
        ),
        headers=parse_headers(table(raw, "headers")),
        rpc=RpcSpec(
            reply_header=str(rpc["reply_header"]),
            deadline_header=str(rpc["deadline_header"]),
            windows_s=MappingProxyType(dict(rpc["windows_s"])),
        ),
        subjects=MappingProxyType(dict(table(raw, "subjects"))),
        max_message_bytes=int(table(raw, "limits")["max_message_bytes"]),
    )


def parse_lane(name: str, spec: Mapping[str, Any]) -> LaneSpec:
    try:
        return LaneSpec(
            name=name,
            stream=str(spec["stream"]),
            durable=str(spec["durable"]),
            filter_subject=str(spec["filter_subject"]),
            done_subject=optional_str(spec["done_subject"]),
            done_msg_id_template=optional_str(spec["done_msg_id_template"]),
            correlation_prefix=str(spec["correlation_prefix"]),
            correlation_separator=str(spec["correlation_separator"]),
            correlation=tuple(spec["correlation"]),
            echo=MappingProxyType(dict(spec["echo"])),
            deadline_s=float(spec["deadline_s"]),
            ack_wait_s=float(spec["ack_wait_s"]),
            heartbeat_s=float(spec["heartbeat_s"]),
            max_deliver=int(spec["max_deliver"]),
            max_ack_pending=int(spec["max_ack_pending"]),
            nak_base_s=optional_float(spec["nak_base_s"]),
            nak_cap_s=optional_float(spec["nak_cap_s"]),
            attempt_window_s=optional_float(spec["attempt_window_s"]),
            quarantine_after_s=optional_float(spec["quarantine_after_s"]),
            bridge_ttl_s=optional_float(spec["bridge_ttl_s"]),
            result_max_bytes=int(spec["result_max_bytes"]),
            result_object_template=optional_str(spec["result_object_template"]),
            reasons=frozenset(spec["reasons"]),
            worker_reasons=frozenset(spec["worker_reasons"]),
            reopenable=frozenset(spec["reopenable"]),
            rpc=bool(spec["rpc"]),
            public=bool(spec["public"]),
        )
    except (KeyError, TypeError, ValueError) as error:
        raise ContractError(f"lane {name}: {error!r}") from error


def parse_stream(name: str, spec: Mapping[str, Any]) -> StreamSpec:
    try:
        return StreamSpec(
            name=name,
            subjects=tuple(spec["subjects"]),
            retention=str(spec["retention"]),
            max_age_s=int(spec["max_age_s"]),
            duplicate_window_s=int(spec["duplicate_window_s"]),
            max_bytes=int(spec["max_bytes"]),
            discard=str(spec["discard"]),
        )
    except (KeyError, TypeError, ValueError) as error:
        raise ContractError(f"stream {name}: {error!r}") from error


def parse_headers(names: Mapping[str, Any]) -> HeaderNames:
    try:
        return HeaderNames(
            msg_id=str(names["msg_id"]),
            reply_to=str(names["reply_to"]),
            deadline=str(names["deadline"]),
            worker_id=str(names["worker_id"]),
            worker_build=str(names["worker_build"]),
            deliveries=str(names["deliveries"]),
            public_node=str(names["public_node"]),
        )
    except KeyError as error:
        raise ContractError(f"headers: no role {error}") from error


def table(raw: Mapping[str, Any], key: str) -> Mapping[str, Any]:
    value = raw.get(key)
    if not isinstance(value, Mapping):
        raise ContractError(f"contract: {key!r} must be an object")
    return value


def optional_str(value: object) -> str | None:
    return None if value is None else str(value)


def optional_float(value: Any) -> float | None:
    return None if value is None else float(value)


def max_utf8_bytes(
    validator: Validator, limit: int, instance: object, schema: Mapping[str, Any]
) -> Iterator[ValidationError]:
    if not isinstance(instance, str):
        return
    try:
        size = len(instance.encode("utf-8"))
    except UnicodeEncodeError:
        yield ValidationError("string is not valid UTF-8")
        return
    if size > limit:
        yield ValidationError(f"{size} UTF-8 bytes is longer than {limit}")


ContractValidator = validators.create(
    meta_schema=Draft202012Validator.META_SCHEMA,
    validators={**Draft202012Validator.VALIDATORS, MAX_UTF8_BYTES: max_utf8_bytes},
    type_checker=Draft202012Validator.TYPE_CHECKER,
    format_checker=Draft202012Validator.FORMAT_CHECKER,
)
