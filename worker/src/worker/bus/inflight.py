from __future__ import annotations

import logging

from worker.bus.lease import Lease, Settled

log = logging.getLogger("worker.bus.inflight")


class Inflight:
    def __init__(self) -> None:
        self._owners: dict[str, Lease] = {}

    async def join(self, lease: Lease) -> Settled | None:
        while True:
            owner = self._owners.get(lease.correlation)
            if owner is None or owner is lease or owner.is_settled:
                self._owners[lease.correlation] = lease
                lease.settled.add_done_callback(lambda _: self._release(lease))
                return None
            log.info(
                "task_joined",
                extra={**lease.log_fields(), "owner_stream_seq": owner.stream_seq},
            )
            if await owner.wait_settled() is Settled.PUBLISHED:
                return Settled.PUBLISHED

    def __len__(self) -> int:
        return sum(1 for owner in self._owners.values() if not owner.is_settled)

    def _release(self, lease: Lease) -> None:
        if self._owners.get(lease.correlation) is lease:
            del self._owners[lease.correlation]
