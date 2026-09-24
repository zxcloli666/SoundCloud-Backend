from __future__ import annotations

import ctypes
import logging
from collections.abc import Mapping

import numpy as np

from worker.runtime.protocol import Arrays, BadInput, SlotSpec

FINGERPRINT = "fingerprint"
LIBRARY = "libchromaprint.so.1"
ALGORITHM_DEFAULT = 1
MAX_CHANNELS = 2
WARMUP_RATE = 11_025
WARMUP_S = 12

log = logging.getLogger(__name__)


class FingerprintSlot:
    def __init__(self) -> None:
        self._library: ctypes.CDLL | None = None

    def load(self, spec: SlotSpec) -> None:
        try:
            self._library = bound(ctypes.CDLL(LIBRARY))
        except OSError as error:
            self._library = None
            log.error("chromaprint unavailable, fingerprints are null", exc_info=error)

    def warmup(self) -> None:
        t = np.arange(WARMUP_RATE * WARMUP_S) / WARMUP_RATE
        tone = (8000 * np.sin(2 * np.pi * 440.0 * t)).astype(np.int16)
        self.invoke(FINGERPRINT, {"pcm": tone}, {"sample_rate": WARMUP_RATE, "channels": 1})

    def invoke(
        self, method: str, arrays: Arrays, args: Mapping[str, object]
    ) -> tuple[Arrays, Mapping[str, object]]:
        if method != FINGERPRINT:
            raise BadInput(f"fingerprint has no method {method!r}")
        pcm = arrays.get("pcm")
        sample_rate = args.get("sample_rate")
        channels = args.get("channels")
        if pcm is None or pcm.ndim != 1 or pcm.dtype != np.int16:
            raise BadInput("pcm must be a one-dimensional int16 array")
        if not isinstance(sample_rate, int) or sample_rate <= 0:
            raise BadInput(f"sample_rate {sample_rate!r} is not a positive integer")
        if not isinstance(channels, int) or not 1 <= channels <= MAX_CHANNELS:
            raise BadInput(f"channels {channels!r} is not 1 or 2")
        if self._library is None:
            return {}, {"fingerprint": None}
        raw = raw_fingerprint(self._library, np.ascontiguousarray(pcm), sample_rate, channels)
        return {}, {"fingerprint": raw}

    def unload(self) -> None:
        self._library = None


def raw_fingerprint(
    library: ctypes.CDLL, pcm: np.ndarray, sample_rate: int, channels: int
) -> str | None:
    context = library.chromaprint_new(ALGORITHM_DEFAULT)
    if not context:
        raise RuntimeError("chromaprint_new failed")
    try:
        expect(library.chromaprint_start(context, sample_rate, channels), "start")
        expect(library.chromaprint_feed(context, pcm.ctypes.data, pcm.size), "feed")
        expect(library.chromaprint_finish(context), "finish")
        values = ctypes.POINTER(ctypes.c_uint32)()
        size = ctypes.c_int()
        expect(
            library.chromaprint_get_raw_fingerprint(
                context, ctypes.byref(values), ctypes.byref(size)
            ),
            "get_raw_fingerprint",
        )
        try:
            return ",".join(str(values[index]) for index in range(size.value)) or None
        finally:
            library.chromaprint_dealloc(values)
    finally:
        library.chromaprint_free(context)


def expect(status: int, call: str) -> None:
    if status != 1:
        raise RuntimeError(f"chromaprint_{call} returned {status}")


def bound(library: ctypes.CDLL) -> ctypes.CDLL:
    context = ctypes.c_void_p
    library.chromaprint_new.restype = context
    library.chromaprint_new.argtypes = [ctypes.c_int]
    library.chromaprint_start.argtypes = [context, ctypes.c_int, ctypes.c_int]
    library.chromaprint_feed.argtypes = [context, ctypes.c_void_p, ctypes.c_int]
    library.chromaprint_finish.argtypes = [context]
    library.chromaprint_get_raw_fingerprint.argtypes = [
        context,
        ctypes.POINTER(ctypes.POINTER(ctypes.c_uint32)),
        ctypes.POINTER(ctypes.c_int),
    ]
    library.chromaprint_dealloc.argtypes = [ctypes.c_void_p]
    library.chromaprint_free.argtypes = [context]
    return library
