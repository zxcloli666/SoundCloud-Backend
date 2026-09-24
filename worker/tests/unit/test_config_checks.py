from __future__ import annotations

import pytest

from tests.conftest import CONFIG_DIR
from worker import settings as s
from worker.contract import Contract


def test_public_trust_allows_only_public_lanes(
    base_env: dict[str, str], contract: Contract
) -> None:
    public = s.load(
        CONFIG_DIR,
        {
            **base_env,
            "WORKER__WORKER__TRUST": "public",
            "WORKER__LANES__ENABLED": '["audio", "lyrics", "transcribe"]',
        },
    )
    s.check_public_lanes(public, contract.public_lanes)
    leaking = s.load(
        CONFIG_DIR,
        {
            **base_env,
            "WORKER__WORKER__TRUST": "public",
            "WORKER__LANES__ENABLED": '["audio", "encode"]',
        },
    )
    with pytest.raises(s.SettingsError, match="non-public lanes \\['encode'\\]"):
        s.check_public_lanes(leaking, contract.public_lanes)


def test_trusted_host_may_enable_every_lane(settings: s.Settings, contract: Contract) -> None:
    s.check_public_lanes(settings, contract.public_lanes)


def test_ping_detects_half_open_sockets_before_any_lease_expires(
    settings: s.Settings, contract: Contract
) -> None:
    ping = settings.nats.ping
    detection_s = ping.interval_s * (ping.max_outstanding + 1)
    slack = min(
        contract.lane(lane).ack_wait_s - contract.lane(lane).heartbeat_s
        for lane in settings.lanes.enabled
    )
    assert detection_s < slack


def test_served_lanes_are_the_contract_lanes(contract: Contract) -> None:
    assert set(contract.lanes) == set(s.LANES)
    assert set(s.LANE_SLOTS) == set(s.LANES)


def test_heartbeat_is_a_fifth_of_ack_wait(contract: Contract) -> None:
    for lane in contract.lanes.values():
        assert lane.heartbeat_s == pytest.approx(lane.ack_wait_s / 5)


def test_cpu_only_slots_are_declared(settings: s.Settings) -> None:
    assert set(settings.slots) >= s.CPU_ONLY_SLOTS
