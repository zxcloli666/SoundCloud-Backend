from __future__ import annotations

import numpy as np
from numpy.typing import NDArray

WINDOWS_PER_S = 100
QUIET_LEVEL = 0.006
QUIET_S = 1.5
TAIL_S = 0.1


class Quiet:
    def __init__(self, rate: int) -> None:
        self._window = rate // WINDOWS_PER_S
        self._quiet_energy = QUIET_LEVEL * QUIET_LEVEL * self._window
        self._limit = round(QUIET_S * WINDOWS_PER_S)
        self._tail_samples = round(TAIL_S * rate)
        self._rest: NDArray[np.float32] = np.zeros(0, np.float32)
        self._seen = 0
        self._run = 0
        self._last_sound: int | None = None

    def fell_silent(self, samples: NDArray[np.float32]) -> bool:
        data = np.concatenate((self._rest, samples))
        whole = data.size // self._window * self._window
        self._rest = data[whole:]
        windows = data[:whole].reshape(-1, self._window)
        energies = np.square(windows).sum(axis=1)
        for energy in energies:
            self._seen += self._window
            if energy < self._quiet_energy:
                self._run += 1
            else:
                self._run = 0
                self._last_sound = self._seen
        return self._last_sound is not None and self._run >= self._limit

    def kept(self) -> int:
        return (self._last_sound or 0) + self._tail_samples
