from __future__ import annotations

import hashlib
import threading
from collections.abc import Iterator
from dataclasses import dataclass, field
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import orjson
import pytest

from worker import fetch_models as fm

BODY = bytes(range(256)) * 20
CUT = 1000
STALL_S = 1.0
TIMEOUT_S = 0.3
DROP = "drop"
STALL = "stall"


@dataclass
class Script:
    body: bytes = BODY
    honor_range: bool = True
    acts: list[str | int] = field(default_factory=list)
    ranges: list[str | None] = field(default_factory=list)


@dataclass
class Served:
    url: str
    script: Script
    pauses: list[float]
    target: Path

    @property
    def partial(self) -> Path:
        return self.target.with_name(self.target.name + fm.PARTIAL_SUFFIX)

    def download(self, sha256: str = hashlib.sha256(BODY).hexdigest()) -> fm.VerifiedDownload:
        return fm.VerifiedDownload(self.url, sha256, self.target)


def handler_for(script: Script) -> type[BaseHTTPRequestHandler]:
    class Files(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            asked = self.headers.get("Range")
            script.ranges.append(asked)
            act = script.acts.pop(0) if script.acts else None
            if act == STALL:
                threading.Event().wait(STALL_S)
                return
            if isinstance(act, int):
                self.send_error(act)
                return
            start = first_byte(asked) if script.honor_range else 0
            if start and start >= len(script.body):
                self.send_error(HTTPStatus.REQUESTED_RANGE_NOT_SATISFIABLE)
                return
            payload = script.body[start:]
            self.answer(start, payload, payload[:CUT] if act == DROP else payload)

        def answer(self, start: int, payload: bytes, sent: bytes) -> None:
            self.send_response(HTTPStatus.PARTIAL_CONTENT if start else HTTPStatus.OK)
            self.send_header("Content-Length", str(len(payload)))
            if start:
                last = len(script.body) - 1
                self.send_header("Content-Range", f"bytes {start}-{last}/{len(script.body)}")
            self.end_headers()
            self.wfile.write(sent)

        def log_message(self, format: str, *args: object) -> None:
            return None

    return Files


def first_byte(asked: str | None) -> int:
    if asked is None:
        return 0
    return int(asked.removeprefix("bytes=").removesuffix("-"))


@pytest.fixture
def served(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[Served]:
    script = Script()
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler_for(script))
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    pauses: list[float] = []
    monkeypatch.setattr(fm.time, "sleep", pauses.append)
    monkeypatch.setattr(fm, "DOWNLOAD_TIMEOUT_S", TIMEOUT_S)
    url = f"http://127.0.0.1:{server.server_address[1]}/voice.gguf"
    yield Served(url, script, pauses, tmp_path / "models" / "voice.gguf")
    server.shutdown()
    server.server_close()


def events(capsys: pytest.CaptureFixture[str]) -> list[dict[str, object]]:
    return [orjson.loads(line) for line in capsys.readouterr().out.splitlines()]


def test_a_dropped_connection_is_resumed_from_the_partial_file(
    served: Served, capsys: pytest.CaptureFixture[str]
) -> None:
    served.script.acts = [DROP]

    failures = fm.fetch_all([fm.Item("tts", served.download())], fm.fetch)

    assert failures == 0
    assert served.target.read_bytes() == BODY
    assert not served.partial.exists()
    assert served.script.ranges == [None, f"bytes={CUT}-"]
    assert served.pauses == [fm.RETRY_PAUSES_S[0]]
    [retry, fetched] = events(capsys)
    assert retry["event"] == "model_download_retry" and retry["kept_bytes"] == CUT
    assert fetched["event"] == "model_fetched"


def test_a_server_that_ignores_the_range_starts_the_file_over(served: Served) -> None:
    served.script.honor_range = False
    served.script.acts = [DROP]

    fm.fetch_download(served.download())

    assert served.target.read_bytes() == BODY
    assert served.script.ranges == [None, f"bytes={CUT}-"]


@pytest.mark.parametrize(
    "status",
    [HTTPStatus.SERVICE_UNAVAILABLE, HTTPStatus.BAD_GATEWAY],
)
def test_a_server_error_is_retried_like_a_network_error(served: Served, status: int) -> None:
    served.script.acts = [status, status]

    fm.fetch_download(served.download())

    assert served.target.read_bytes() == BODY
    assert served.pauses == list(fm.RETRY_PAUSES_S[:2])


@pytest.mark.parametrize("status", [HTTPStatus.NOT_FOUND, HTTPStatus.FORBIDDEN])
def test_a_file_the_server_refuses_fails_at_once(served: Served, status: int) -> None:
    served.script.acts = [status]

    with pytest.raises(fm.FetchError, match=r"\(attempts: 1\)"):
        fm.fetch_download(served.download())

    assert served.pauses == []
    assert served.script.ranges == [None]
    assert not served.target.exists()


@pytest.mark.parametrize(
    "status",
    [HTTPStatus.TOO_MANY_REQUESTS, HTTPStatus.REQUEST_TIMEOUT, HTTPStatus.TOO_EARLY],
)
def test_a_busy_or_slow_answer_is_retried(served: Served, status: int) -> None:
    served.script.acts = [status]

    fm.fetch_download(served.download())

    assert served.target.read_bytes() == BODY
    assert served.pauses == [fm.RETRY_PAUSES_S[0]]
    assert len(served.script.ranges) == 2


def test_a_stalled_server_times_out_and_is_retried(served: Served) -> None:
    served.script.acts = [STALL]

    fm.fetch_download(served.download())

    assert served.target.read_bytes() == BODY
    assert len(served.script.ranges) == 2


def test_the_pauses_grow_and_the_file_fails_only_after_every_retry(served: Served) -> None:
    served.script.acts = [DROP, *[HTTPStatus.SERVICE_UNAVAILABLE] * len(fm.RETRY_PAUSES_S)]

    with pytest.raises(fm.FetchError, match="attempts"):
        fm.fetch_download(served.download())

    assert served.pauses == list(fm.RETRY_PAUSES_S)
    assert served.pauses == sorted(served.pauses)
    assert served.partial.read_bytes() == BODY[:CUT]
    assert not served.target.exists()
    assert served.script.ranges[1:] == [f"bytes={CUT}-"] * len(fm.RETRY_PAUSES_S)


def test_a_resumed_file_with_a_wrong_hash_is_fetched_once_more_from_zero(
    served: Served, capsys: pytest.CaptureFixture[str]
) -> None:
    served.partial.parent.mkdir(parents=True)
    served.partial.write_bytes(b"x" * CUT)

    fm.fetch_download(served.download())

    assert served.target.read_bytes() == BODY
    assert served.script.ranges == [f"bytes={CUT}-", None]
    assert [event["event"] for event in events(capsys)] == ["model_download_restarted"]


def test_a_file_that_keeps_failing_the_hash_is_deleted_and_reported(served: Served) -> None:
    served.partial.parent.mkdir(parents=True)
    served.partial.write_bytes(b"x" * CUT)

    with pytest.raises(fm.FetchError, match="sha256"):
        fm.fetch_download(served.download(sha256="0" * 64))

    assert served.script.ranges == [f"bytes={CUT}-", None]
    assert list(served.target.parent.iterdir()) == []


def test_a_fresh_file_with_a_wrong_hash_is_not_fetched_again(served: Served) -> None:
    with pytest.raises(fm.FetchError, match="sha256"):
        fm.fetch_download(served.download(sha256="0" * 64))

    assert served.script.ranges == [None]
    assert list(served.target.parent.iterdir()) == []


def test_a_partial_file_that_is_already_whole_is_kept(served: Served) -> None:
    served.partial.parent.mkdir(parents=True)
    served.partial.write_bytes(BODY)

    fm.fetch_download(served.download())

    assert served.target.read_bytes() == BODY
    assert served.script.ranges == [f"bytes={len(BODY)}-"]
