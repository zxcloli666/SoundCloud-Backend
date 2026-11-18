from __future__ import annotations

import base64

from worker.contract import Contract
from worker.domain.video import publish


def test_a_file_splits_into_parts_that_join_back_to_it() -> None:
    blob = bytes(range(256)) * 20

    parts = publish.split(blob, 1000)

    assert [len(part) for part in parts] == [1000] * 5 + [120]
    assert b"".join(parts) == blob


def test_an_empty_file_has_no_parts() -> None:
    assert publish.split(b"", 1000) == []


def test_a_full_part_in_base64_leaves_room_for_the_verdict_in_one_message(
    contract: Contract,
) -> None:
    part = bytes(contract.video_limits.part_bytes)

    encoded = len(base64.b64encode(part))

    assert encoded + 4096 <= contract.max_message_bytes
    assert encoded + 4096 <= contract.lane("video-find").result_max_bytes
