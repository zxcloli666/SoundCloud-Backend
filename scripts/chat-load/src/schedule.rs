use std::time::Duration;

use anyhow::bail;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Schedule {
    pub steps: Vec<usize>,
    pub step: Duration,
    pub spread: Duration,
    pub hold: Duration,
    pub restart_at: Option<Duration>,
    pub worst: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Phase {
    Ramp(usize),
    Hold,
    Worst,
    Done,
}

impl Schedule {
    pub fn check(&self, devices: usize) -> anyhow::Result<()> {
        if self.steps.is_empty() || self.steps.windows(2).any(|pair| pair[0] >= pair[1]) {
            bail!("steps must be a rising list of device counts");
        }
        if self.steps[0] == 0 {
            bail!("the first step brings no device online");
        }
        if self.peak() > devices {
            bail!(
                "the last step wants {} devices and the layout has {devices}",
                self.peak()
            );
        }
        if self.spread > self.step {
            bail!("the spread is longer than a step");
        }
        if let Some(at) = self.restart_at
            && at >= self.hold
        {
            bail!("the restart is not inside the hold");
        }
        Ok(())
    }

    pub fn peak(&self) -> usize {
        self.steps.last().copied().unwrap_or(0)
    }

    pub fn hold_starts(&self) -> Duration {
        self.step * self.steps.len() as u32
    }

    pub fn worst_starts(&self) -> Duration {
        self.hold_starts() + self.hold
    }

    pub fn total(&self) -> Duration {
        self.worst_starts() + self.worst
    }

    pub fn restart(&self) -> Option<Duration> {
        self.restart_at.map(|at| self.hold_starts() + at)
    }

    pub fn launch_at(&self, position: usize) -> Option<Duration> {
        let step = self.steps.iter().position(|count| position < *count)?;
        let before = match step {
            0 => 0,
            _ => self.steps[step - 1],
        };
        let joining = (self.steps[step] - before) as f64;
        let share = (position - before) as f64 / joining;
        Some(self.step * step as u32 + self.spread.mul_f64(share))
    }

    pub fn phase(&self, at: Duration) -> Phase {
        if at >= self.total() {
            return Phase::Done;
        }
        if at >= self.worst_starts() {
            return Phase::Worst;
        }
        if at >= self.hold_starts() {
            return Phase::Hold;
        }
        Phase::Ramp((at.as_secs_f64() / self.step.as_secs_f64()) as usize)
    }
}

#[cfg(test)]
#[path = "tests/schedule_tests.rs"]
mod tests;
