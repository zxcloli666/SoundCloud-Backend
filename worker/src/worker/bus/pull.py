from __future__ import annotations

import json
from collections.abc import AsyncIterator

import nats.errors
import nats.js.errors
from nats.aio.msg import Msg
from nats.js import JetStreamContext, api

from worker.domain.deadline import Clock

NANOS_PER_S = 1_000_000_000
REPLY_SLACK_S = 1.0
CLOSING_STATUSES = frozenset(
    {api.StatusCode.NO_MESSAGES, api.StatusCode.REQUEST_TIMEOUT, api.StatusCode.CONFLICT}
)


async def pulled(
    subscription: JetStreamContext.PullSubscription, batch: int, expires_s: float, clock: Clock
) -> AsyncIterator[Msg]:
    request = {"batch": batch, "expires": int(expires_s * NANOS_PER_S), "no_wait": True}
    await subscription._nc.publish(
        subscription._nms, json.dumps(request).encode(), subscription._deliver
    )
    until = clock.now() + expires_s + REPLY_SLACK_S
    taken = 0
    while taken < batch and clock.now() < until:
        try:
            msg = await subscription._sub.next_msg(timeout=until - clock.now())
        except nats.errors.TimeoutError:
            return
        status = JetStreamContext.is_status_msg(msg)
        if status in CLOSING_STATUSES:
            return
        if status is not None:
            raise nats.js.errors.APIError.from_msg(msg)
        taken += 1
        yield msg
