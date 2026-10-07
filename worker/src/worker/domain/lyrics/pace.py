from __future__ import annotations

from dataclasses import dataclass

SMOOTHING = 0.3
SKIP_DECAY = 0.9
MISS_MARGIN = 1.25


@dataclass
class Pace:
    seconds_per_audio_s: float = 0.0

    def fits(self, audio_s: float, budget_s: float) -> bool:
        return self.seconds_per_audio_s * audio_s <= budget_s

    def observe(self, audio_s: float, elapsed_s: float) -> None:
        rate = elapsed_s / max(audio_s, 1e-3)
        if self.seconds_per_audio_s == 0.0:
            self.seconds_per_audio_s = rate
            return
        self.seconds_per_audio_s += SMOOTHING * (rate - self.seconds_per_audio_s)

    def missed(self, audio_s: float, budget_s: float) -> None:
        floor = MISS_MARGIN * budget_s / max(audio_s, 1e-3)
        self.seconds_per_audio_s = max(self.seconds_per_audio_s, floor)

    def skipped(self) -> None:
        self.seconds_per_audio_s *= SKIP_DECAY
