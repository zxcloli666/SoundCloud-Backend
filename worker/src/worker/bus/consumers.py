from __future__ import annotations

import asyncio
import logging
import random
from collections.abc import Mapping
from enum import Enum, StrEnum

import nats.errors
import nats.js.errors
from nats.js import JetStreamContext, api

from worker.bus.connection import Connection
from worker.contract import Contract, LaneSpec
from worker.domain.deadline import Clock
from worker.observability.counters import Counters
from worker.settings import Settings, check_public_lanes

log = logging.getLogger("worker.bus.consumers")

EX_CONFIG = 78
CHECK_INTERVAL_S = 60.0
NOT_SERVED_RETRY_S = 600.0
UNAVAILABLE_RETRY_MIN_S = 10.0
UNAVAILABLE_RETRY_MAX_S = 60.0
CONSUMER_INFO_TIMEOUT_S = 5.0
DEFAULT_ACK_WAIT_S = 30.0
DEFAULT_MAX_ACK_PENDING = 1000
DEFAULT_MAX_DELIVER = -1
BROKER_MAX_DELIVER = 1
JS_API_PREFIX = "$JS.API"


class LaneState(StrEnum):
    SERVING = "serving"
    NOT_PROVISIONED = "not_provisioned"
    NOT_SERVED = "not_served"
    DEGRADED = "degraded"
    PAUSED = "paused"
    DRAINING = "draining"


class ConfigDrift(Exception):
    exit_code = EX_CONFIG

    def __init__(self, lane: str, diff: Mapping[str, tuple[object, object]]) -> None:
        super().__init__(f"consumer of lane {lane} drifted from the contract: {dict(diff)}")
        self.lane = lane
        self.diff = dict(diff)


def served_lanes(settings: Settings, contract: Contract) -> tuple[LaneSpec, ...]:
    check_public_lanes(settings, contract.public_lanes)
    return tuple(contract.lane(name) for name in settings.lanes.enabled)


def expected_max_deliver(lane: LaneSpec, public: bool) -> int:
    return BROKER_MAX_DELIVER if public else lane.max_deliver


def consumer_diff(
    lane: LaneSpec, actual: api.ConsumerConfig, public: bool = False
) -> dict[str, tuple[object, object]]:
    expected: dict[str, object] = {
        "filter_subject": lane.filter_subject,
        "ack_policy": api.AckPolicy.EXPLICIT.value,
        "deliver_policy": api.DeliverPolicy.ALL.value,
        "ack_wait_s": lane.ack_wait_s,
        "max_deliver": expected_max_deliver(lane, public),
        "max_ack_pending": lane.max_ack_pending,
    }
    observed: dict[str, object] = {
        "filter_subject": observed_filter(actual),
        "ack_policy": policy_value(actual.ack_policy, api.AckPolicy.EXPLICIT.value),
        "deliver_policy": policy_value(actual.deliver_policy, api.DeliverPolicy.ALL.value),
        "ack_wait_s": float(actual.ack_wait) if actual.ack_wait else DEFAULT_ACK_WAIT_S,
        "max_deliver": observed_max_deliver(actual),
        "max_ack_pending": actual.max_ack_pending or DEFAULT_MAX_ACK_PENDING,
    }
    return {
        key: (expected[key], observed[key]) for key in expected if expected[key] != observed[key]
    }


def observed_filter(config: api.ConsumerConfig) -> str:
    if config.filter_subject:
        return str(config.filter_subject)
    if config.filter_subjects and len(config.filter_subjects) == 1:
        return str(config.filter_subjects[0])
    return ""


def observed_max_deliver(config: api.ConsumerConfig) -> int:
    return DEFAULT_MAX_DELIVER if not config.max_deliver else int(config.max_deliver)


def policy_value(policy: object, default: str) -> str:
    if isinstance(policy, Enum):
        return str(policy.value)
    return str(policy) if policy else default


class ConsumerWatch:
    def __init__(
        self,
        lane: LaneSpec,
        js: JetStreamContext,
        connection: Connection,
        counters: Counters,
        clock: Clock,
        required: bool,
        required_grace_s: float,
        public: bool = False,
    ) -> None:
        self.lane = lane
        self.required = required
        self.required_grace_s = required_grace_s
        self.public = public
        self.max_deliver = expected_max_deliver(lane, public)
        self.drifted = asyncio.Event()
        self.diff: dict[str, tuple[object, object]] = {}
        self.last_error: str | None = None
        self.paused = False
        self.engine_broken = False
        self.publish_blocked = False
        self.retry_after_s = CHECK_INTERVAL_S
        self._js = js
        self._connection = connection
        self._counters = counters
        self._clock = clock
        self._consumer_state = LaneState.NOT_PROVISIONED
        self._not_serving_since: float | None = clock.now()
        self._degraded_noted = False

    @property
    def info_subject(self) -> str:
        return f"{JS_API_PREFIX}.CONSUMER.INFO.{self.lane.stream}.{self.lane.durable}"

    @property
    def state(self) -> LaneState:
        if self.drifted.is_set():
            return LaneState.DRAINING
        if self.publish_blocked or self.engine_broken or self.required_missing:
            return LaneState.DEGRADED
        if self.paused:
            return LaneState.PAUSED
        return self._consumer_state

    @property
    def required_missing(self) -> bool:
        if not self.required or self._not_serving_since is None:
            return False
        return self._clock.now() - self._not_serving_since > self.required_grace_s

    async def verify_at_start(self) -> LaneState:
        state = await self.check()
        if self.drifted.is_set():
            raise ConfigDrift(self.lane.name, self.diff)
        return state

    async def run(self) -> None:
        while True:
            await self._connection.wait_connected()
            await self.check()
            if self.drifted.is_set():
                return
            await self._clock.sleep(self.retry_after_s)

    async def check(self) -> LaneState:
        started = self._clock.now()
        try:
            info = await self._js.consumer_info(
                self.lane.stream, self.lane.durable, timeout=CONSUMER_INFO_TIMEOUT_S
            )
        except nats.js.errors.NotFoundError as error:
            self._observe(LaneState.NOT_PROVISIONED, CHECK_INTERVAL_S, str(error))
        except nats.js.errors.ServiceUnavailableError as error:
            self._keep(jittered_retry_s(), str(error))
        except nats.errors.TimeoutError as error:
            self._observe_timeout(started, error)
        except nats.js.errors.APIError as error:
            self._keep(CHECK_INTERVAL_S, str(error))
        except nats.errors.Error as error:
            self._keep(UNAVAILABLE_RETRY_MIN_S, str(error) or type(error).__name__)
        else:
            self._connection.trust()
            self._observe_config(info.config)
        self._note_degraded()
        return self.state

    def _observe_timeout(self, started: float, error: Exception) -> None:
        if not self._connection.is_connected:
            self._keep(UNAVAILABLE_RETRY_MIN_S, "disconnected")
        elif self._connection.permission_denied(self.info_subject, started):
            self._observe(LaneState.NOT_SERVED, NOT_SERVED_RETRY_S, "permissions violation")
        else:
            self._keep(jittered_retry_s(), str(error) or "consumer_info timed out")

    def _observe_config(self, config: api.ConsumerConfig) -> None:
        diff = consumer_diff(self.lane, config, self.public)
        if diff:
            self.diff = diff
            self.last_error = f"consumer_config_mismatch: {diff}"
            self._counters.inc("consumer_config_mismatch_total", lane=self.lane.name)
            log.error(
                "consumer_config_mismatch",
                extra={"lane": self.lane.name, "durable": self.lane.durable, "diff": diff},
            )
            self.drifted.set()
            return
        self.max_deliver = observed_max_deliver(config)
        self._observe(LaneState.SERVING, CHECK_INTERVAL_S, None)

    def _observe(self, state: LaneState, retry_after_s: float, error: str | None) -> None:
        self.retry_after_s = retry_after_s
        self.last_error = error
        if state is self._consumer_state:
            return
        log.info(
            "lane_consumer_state",
            extra={
                "lane": self.lane.name,
                "from": self._consumer_state,
                "to": state,
                "error": error,
            },
        )
        self._counters.inc("lane_state_changes_total", lane=self.lane.name, state=state)
        self._consumer_state = state
        if state is LaneState.SERVING:
            self._not_serving_since = None
            self._degraded_noted = False
        elif self._not_serving_since is None:
            self._not_serving_since = self._clock.now()

    def _keep(self, retry_after_s: float, error: str) -> None:
        self.retry_after_s = retry_after_s
        self.last_error = error
        self._counters.inc("consumer_check_failures_total", lane=self.lane.name)
        log.warning(
            "consumer_check_failed",
            extra={"lane": self.lane.name, "state": self._consumer_state, "error": error},
        )

    def _note_degraded(self) -> None:
        if self.required_missing and not self._degraded_noted:
            self._degraded_noted = True
            self._counters.inc("lane_required_missing_total", lane=self.lane.name)
            log.error(
                "lane_required_missing",
                extra={
                    "lane": self.lane.name,
                    "state": self._consumer_state,
                    "error": self.last_error,
                },
            )


def jittered_retry_s() -> float:
    return random.uniform(UNAVAILABLE_RETRY_MIN_S, UNAVAILABLE_RETRY_MAX_S)
