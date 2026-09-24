from __future__ import annotations

import asyncio
from dataclasses import dataclass, field

from aiohttp import web
from aiohttp.test_utils import TestServer


@dataclass
class FakeAudioServer:
    body: bytes = b""
    slow_chunk_delay_s: float = 0.2
    huge_bytes: int = 100 * 1024 * 1024
    requests: list[str] = field(default_factory=list)
    _server: TestServer | None = None

    async def __aenter__(self) -> FakeAudioServer:
        app = web.Application()
        app.add_routes(
            [
                web.get("/ok.wav", self._ok),
                web.get("/missing", self._status(404)),
                web.get("/gone", self._status(410)),
                web.get("/forbidden", self._status(403)),
                web.get("/unauthorized", self._status(401)),
                web.get("/error", self._status(500)),
                web.get("/slow", self._slow),
                web.get("/huge", self._huge),
                web.get("/garbage", self._garbage),
            ]
        )
        self._server = TestServer(app)
        await self._server.start_server()
        return self

    async def __aexit__(self, *exc: object) -> None:
        if self._server is not None:
            await self._server.close()

    def url(self, path: str) -> str:
        if self._server is None:
            raise RuntimeError("server not started")
        return str(self._server.make_url(path))

    async def _ok(self, request: web.Request) -> web.Response:
        self.requests.append(request.path)
        return web.Response(body=self.body, content_type="audio/wav")

    def _status(self, status: int):
        async def handler(request: web.Request) -> web.Response:
            self.requests.append(request.path)
            return web.Response(status=status)

        return handler

    async def _slow(self, request: web.Request) -> web.StreamResponse:
        self.requests.append(request.path)
        response = web.StreamResponse()
        await response.prepare(request)
        for offset in range(0, len(self.body), 4096):
            await response.write(self.body[offset : offset + 4096])
            await asyncio.sleep(self.slow_chunk_delay_s)
        await response.write_eof()
        return response

    async def _huge(self, request: web.Request) -> web.StreamResponse:
        self.requests.append(request.path)
        response = web.StreamResponse(headers={"Content-Length": str(self.huge_bytes)})
        await response.prepare(request)
        chunk = b"\0" * (1024 * 1024)
        written = 0
        while written < self.huge_bytes:
            await response.write(chunk)
            written += len(chunk)
        await response.write_eof()
        return response

    async def _garbage(self, request: web.Request) -> web.Response:
        self.requests.append(request.path)
        return web.Response(body=b"not audio at all", content_type="audio/mpeg")
