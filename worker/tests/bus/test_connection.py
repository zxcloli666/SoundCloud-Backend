from __future__ import annotations

import logging

import nats.errors
import pytest

from tests.bus.conftest import Harness
from worker.bus.connection import PROBE_INTERVAL_S


async def test_probe_trusts_a_suspicious_connection_on_the_first_js_answer(
    harness: Harness,
) -> None:
    harness.connection.suspect("ack_sync")
    harness.bus.api_faults.append(nats.errors.TimeoutError())
    await harness.settle()
    await harness.clock.tick(PROBE_INTERVAL_S)
    assert harness.connection.suspicious
    assert harness.counters.value("nats_errors_total", kind="probe") == 1
    await harness.clock.tick(PROBE_INTERVAL_S)
    assert not harness.connection.suspicious
    assert harness.bus.account_infos == 1


async def test_probe_waits_for_reconnect_before_asking(harness: Harness) -> None:
    await harness.bus.disconnect()
    harness.connection.suspect("puback")
    await harness.clock.tick(PROBE_INTERVAL_S * 3)
    assert harness.connection.suspicious
    assert harness.bus.account_infos == 0
    await harness.bus.reconnect()
    await harness.settle()
    await harness.clock.tick(PROBE_INTERVAL_S)
    assert not harness.connection.suspicious


async def test_planned_close_logs_nats_closed_as_info(
    harness: Harness, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.INFO, logger="worker.bus.connection")
    await harness.connection.close()
    assert harness.connection.closed.is_set()
    assert closed_levels(caplog) == [logging.INFO]


async def test_unexpected_close_logs_nats_closed_as_error(
    harness: Harness, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.INFO, logger="worker.bus.connection")
    await harness.bus.close()
    assert harness.connection.closed.is_set()
    assert closed_levels(caplog) == [logging.ERROR]


def closed_levels(caplog: pytest.LogCaptureFixture) -> list[int]:
    return [record.levelno for record in caplog.records if record.message == "nats_closed"]
