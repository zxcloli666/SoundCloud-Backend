from __future__ import annotations

import json
import logging
import random
import time
import urllib.parse
import urllib.request
from urllib.error import HTTPError

from eval.collect import USER_AGENT

BASE = "https://lrclib.net/api"
PAUSE_S = 0.3
ATTEMPTS = 4
TIMEOUT_S = 30
RETRY_STATUSES = (429, 503)
DEFAULT_RETRY_S = 5.0
MAX_RETRY_S = 120.0
JITTER_S = 1.0

log = logging.getLogger("eval.lrclib")


class LrclibError(RuntimeError):
    pass


def get_json(path: str, params: dict[str, str] | None = None) -> object:
    query = f"?{urllib.parse.urlencode(params)}" if params else ""
    request = urllib.request.Request(
        f"{BASE}/{path}{query}", headers={"User-Agent": USER_AGENT, "Accept": "application/json"}
    )
    for attempt in range(ATTEMPTS):
        time.sleep(PAUSE_S)
        try:
            with urllib.request.urlopen(request, timeout=TIMEOUT_S) as response:
                content_type = response.headers.get("Content-Type", "")
                if "json" not in content_type:
                    raise LrclibError(f"{path}: content type {content_type or 'missing'}")
                return json.loads(response.read().decode("utf-8"))
        except HTTPError as error:
            if error.code not in RETRY_STATUSES or attempt + 1 == ATTEMPTS:
                raise LrclibError(f"{path}: http {error.code}") from error
            wait = retry_after(error) + random.uniform(0.0, JITTER_S)
            log.warning("lrclib %s: http %d, waiting %.1fs", path, error.code, wait)
            time.sleep(wait)
        except ValueError as error:
            raise LrclibError(f"{path}: body is not json") from error
        except OSError as error:
            log.warning("lrclib %s attempt %d failed: %s", path, attempt + 1, error)
            time.sleep(2.0 * (attempt + 1))
    raise LrclibError(f"{path}: unreachable")


def retry_after(error: HTTPError) -> float:
    try:
        return min(MAX_RETRY_S, float(error.headers.get("Retry-After", "")))
    except ValueError:
        return DEFAULT_RETRY_S
