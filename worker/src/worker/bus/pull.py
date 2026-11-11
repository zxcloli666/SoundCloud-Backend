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


class PullUnanswered(Exception):
    pass


async def pulled(
    subscription: JetStreamContext.PullSubscription, batch: int, expires_s: float, clock: Clock
) -> AsyncIterator[Msg]:
    request = {"batch": batch, "expires": int(expires_s * NANOS_PER_S)}
    await subscription._nc.publish(
        subscription._nms, json.dumps(request).encode(), subscription._deliver
    )
    until = clock.now() + expires_s + REPLY_SLACK_S
    for _ in range(batch):
        try:
            msg = await subscription._sub.next_msg(timeout=until - clock.now())
        except nats.errors.TimeoutError:
            raise PullUnanswered(f"no reply within {expires_s + REPLY_SLACK_S} s") from None
        status = JetStreamContext.is_status_msg(msg)
        if status == api.StatusCode.REQUEST_TIMEOUT:
            return
        if status is not None:
            raise nats.js.errors.APIError(
                code=int(status), description=(msg.headers or {}).get(api.Header.DESCRIPTION)
            )
        yield msg
