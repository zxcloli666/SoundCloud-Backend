from __future__ import annotations

import logging
from collections.abc import Iterator

import pytest
from eval.harness import LANE_LOGGER, MetricCapture, configure_logging


@pytest.fixture
def capture() -> Iterator[MetricCapture]:
    root = logging.getLogger()
    lane = logging.getLogger(LANE_LOGGER)
    saved = (list(root.handlers), root.level, lane.level)
    root.handlers.clear()
    handler = MetricCapture()
    configure_logging()
    lane.addHandler(handler)
    yield handler
    lane.removeHandler(handler)
    root.handlers[:] = saved[0]
    root.setLevel(saved[1])
    lane.setLevel(saved[2])


def test_line_features_logged_at_debug_reach_the_capture(capture: MetricCapture) -> None:
    lane = logging.getLogger(LANE_LOGGER)
    lane.info("lyrics synced", extra={"inside_share": 0.9, "anchor_agreement": 0.7})
    lane.debug("line features", extra={"line_features": {"3": [1.0, 0.5]}})
    assert capture.last == {"inside_share": 0.9, "anchor_agreement": 0.7}
    assert capture.line_features == {3: [1.0, 0.5]}


def test_console_stays_at_info(capture: MetricCapture) -> None:
    console = logging.getLogger().handlers[0]
    assert console.level == logging.INFO
