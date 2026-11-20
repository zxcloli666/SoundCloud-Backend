use std::time::Duration;

use tracing::info;

use super::director::{self, Director, Look, Replan};
use super::flow::{Directed, Flow};
use super::lanes::Nodes;
use super::state::{self, Gen, Save, Stage};
use super::{Store, VideoGen};
use crate::handlers::video_accounts::Notify;
use crate::queue::{JobError, JobResult};

impl<D: Director, S: Store, L: Nodes, N: Notify> VideoGen<D, S, L, N> {
    pub(super) async fn redo(
        &self,
        mut job: Gen,
        look: Look,
        reason: &str,
        edit: &str,
    ) -> JobResult {
        let (name, tries) = match look {
            Look::Keyframe => ("keyframe", self.config.keyframe_tries),
            Look::Shot => ("shot", self.config.shot_tries),
        };
        job.reject(name, reason, edit);
        self.keep_reviewed(&mut job, look).await;
        job.redo += 1;
        job.state.redos += 1;
        let shot = job.shot + 1;
        if job.state.redos >= self.config.track_redos {
            let redos = job.state.redos;
            let why = format!("{redos} redos spent, the last on {name} {shot}: {reason}");
            return self.give_up(job, name, &why).await;
        }
        if job.redo < tries {
            job.state.edit = Some(edit.to_owned());
            job.stage = drawn(look);
            return self.send(job).await;
        }
        if job.state.replans >= self.config.shot_replans {
            let replans = job.state.replans;
            let why = format!("{name} {shot} failed after {replans} re-plans: {reason}");
            return self.give_up(job, name, &why).await;
        }
        info!(track = %job.track, name, shot, "generated shot is planned again");
        self.keep_reviewed(&mut job, Look::Keyframe).await;
        job.state.frames.truncate(job.shot as usize);
        job.state.replans += 1;
        job.state.edit = None;
        job.redo = 0;
        job.stage = Stage::Plan;
        let again = Save {
            sent: false,
            wait: Duration::ZERO,
        };
        state::save(&self.pool, &job, job.step, again).await?;
        Ok(())
    }

    pub(super) async fn replan(&self, mut job: Gen) -> JobResult<Flow> {
        let mut plan = job
            .plan
            .clone()
            .ok_or_else(|| JobError::permanent(anyhow::anyhow!("a re-plan without a plan")))?;
        let shot = job.shot as usize;
        let request = Replan {
            plan: &plan,
            shot,
            cover_url: &job.state.cover,
            failures: job.failures(),
        };
        let answer = self.director.replan(&request).await;
        let reshot = match self.directed(&job, answer).await? {
            Directed::Done(reshot) => director::reshot(reshot),
            Directed::Halted => return Ok(Flow::Halted),
            Directed::Invalid(reason) => Err(reason),
        };
        match reshot {
            Ok(reshot) => {
                plan.shots[shot].keyframe = reshot.keyframe;
                plan.shots[shot].motion = reshot.motion;
                job.plan = Some(plan);
                job.stage = Stage::Keyframe;
                job.faults = 0;
                self.send(job).await?;
            }
            Err(reason) => self.fault(job, &format!("re-plan: {reason}")).await?,
        }
        Ok(Flow::Moved)
    }

    async fn give_up(&self, mut job: Gen, name: &str, why: &str) -> JobResult {
        let shot = job.shot + 1;
        job.end(Stage::Rejected, why);
        info!(track = %job.track, name, shot, "generated video rejected");
        self.close(job).await
    }
}

pub(super) fn drawn(look: Look) -> Stage {
    match look {
        Look::Keyframe => Stage::Keyframe,
        Look::Shot => Stage::Shot,
    }
}
