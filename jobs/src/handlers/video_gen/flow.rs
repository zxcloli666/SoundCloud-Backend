use std::time::Duration;

use tracing::{info, warn};

use super::director::{self, Brief, Director, Look, Picture, Review, Verdict};
use super::lanes::{self, Nodes};
use super::state::{self, Accepted, Gen, Save, Stage};
use super::{LADDER, Store, VideoGen};
use crate::handlers::video_accounts::Notify;
use crate::llm::anthropic::{Answer, LlmError};
use crate::llm::ledger;
use crate::queue::{JobError, JobResult};

const RETRY_PUBLISH: Duration = Duration::from_secs(120);

pub enum Flow {
    Moved,
    Halted,
}

enum Directed<T> {
    Done(T),
    Halted,
    Invalid(String),
}

impl<D: Director, S: Store, L: Nodes, N: Notify> VideoGen<D, S, L, N> {
    pub(super) async fn advance(&self, job: Gen) -> JobResult<Flow> {
        match job.stage {
            Stage::Plan => self.plan(job).await,
            Stage::Keyframe | Stage::Shot | Stage::Assemble => {
                self.send(job).await?;
                Ok(Flow::Moved)
            }
            Stage::KeyframeReview => self.review(job, Look::Keyframe).await,
            Stage::ShotReview => self.review(job, Look::Shot).await,
            Stage::Done | Stage::Rejected | Stage::Failed => Ok(Flow::Moved),
        }
    }

    async fn plan(&self, mut job: Gen) -> JobResult<Flow> {
        if !self.affordable().await? {
            return Ok(Flow::Halted);
        }
        let Some(brief) = self.brief(&job.track).await? else {
            job.end(Stage::Failed, "the track has no cover or duration");
            self.close(job).await?;
            return Ok(Flow::Moved);
        };
        let answer = self.director.plan(&brief).await;
        let plan = match self.directed(&job, answer).await? {
            Directed::Done(plan) => director::checked(plan, &brief),
            Directed::Halted => return Ok(Flow::Halted),
            Directed::Invalid(reason) => Err(reason),
        };
        match plan {
            Ok(plan) => {
                info!(track = %job.track, mood = %plan.mood, "video plan written");
                job.plan = Some(plan);
                job.state.cover = brief.cover_url;
                job.stage = Stage::Keyframe;
                job.faults = 0;
                self.send(job).await?;
            }
            Err(reason) => {
                job.reject("plan", &reason, "");
                self.fault(job, &format!("plan: {reason}")).await?;
            }
        }
        Ok(Flow::Moved)
    }

    async fn review(&self, mut job: Gen, look: Look) -> JobResult<Flow> {
        if !self.affordable().await? {
            return Ok(Flow::Halted);
        }
        let plan = job
            .plan
            .clone()
            .ok_or_else(|| JobError::permanent(anyhow::anyhow!("a review without a plan")))?;
        let keys = match look {
            Look::Keyframe => job.state.frame.iter().cloned().collect(),
            Look::Shot => job.state.stills.clone(),
        };
        let Some(current) = self.pictures(&keys, "frame").await? else {
            job.stage = drawn(look);
            self.send(job).await?;
            return Ok(Flow::Moved);
        };
        let shot = job.shot as usize;
        let seen = match look {
            Look::Keyframe => shot,
            Look::Shot => shot + 1,
        };
        let earlier = self
            .pictures(
                &job.state.frames[..job.state.frames.len().min(seen)],
                "keyframe",
            )
            .await?
            .unwrap_or_default();
        let review = Review {
            look,
            plan: &plan,
            shot,
            cover_url: &job.state.cover,
            current,
            earlier,
        };
        let answer = self.director.review(&review).await;
        let verdict = match self.directed(&job, answer).await? {
            Directed::Done(verdict) => verdict,
            Directed::Halted => return Ok(Flow::Halted),
            Directed::Invalid(reason) => {
                self.fault(job, &format!("review: {reason}")).await?;
                return Ok(Flow::Moved);
            }
        };
        match (verdict, look) {
            (Verdict::Accept { .. }, Look::Keyframe) => self.keyframe_accepted(job).await?,
            (Verdict::Accept { trim_start_ms }, Look::Shot) => {
                self.shot_accepted(job, trim_start_ms).await?
            }
            (Verdict::Redo { reason, edit }, _) => self.redo(job, look, &reason, &edit).await?,
        }
        Ok(Flow::Moved)
    }

    async fn keyframe_accepted(&self, mut job: Gen) -> JobResult {
        let frame = job.state.frame.clone().unwrap_or_default();
        job.state.frames.truncate(job.shot as usize);
        job.state.frames.push(frame);
        job.state.edit = None;
        job.redo = 0;
        job.stage = Stage::Shot;
        self.send(job).await
    }

    async fn shot_accepted(&self, mut job: Gen, trim_start_ms: i32) -> JobResult {
        let shots = job.plan.as_ref().map_or(0, |plan| plan.shots.len());
        let planned = job.plan.as_ref().map_or(0, |plan| {
            plan.shots[job.shot as usize].end_ms - plan.shots[job.shot as usize].start_ms
        });
        let room = (self.config.shot_ms() - planned).max(0);
        let key = job.state.shot.clone().unwrap_or_default();
        job.state.accepted.truncate(job.shot as usize);
        job.state.accepted.push(Accepted {
            key,
            trim_start_ms: trim_start_ms.clamp(0, room),
        });
        let stills = std::mem::take(&mut job.state.stills);
        self.drop_temps(&stills).await;
        job.state.temps.retain(|key| !stills.contains(key));
        job.state.edit = None;
        job.redo = 0;
        job.shot += 1;
        job.stage = match job.shot as usize >= shots {
            true => Stage::Assemble,
            false => Stage::Keyframe,
        };
        self.send(job).await
    }

    async fn redo(&self, mut job: Gen, look: Look, reason: &str, edit: &str) -> JobResult {
        let (name, tries) = match look {
            Look::Keyframe => ("keyframe", self.config.keyframe_tries),
            Look::Shot => ("shot", self.config.shot_tries),
        };
        job.reject(name, reason, edit);
        job.redo += 1;
        if job.redo >= tries {
            let shot = job.shot + 1;
            let redo = job.redo;
            job.end(
                Stage::Rejected,
                &format!("{name} {shot} rejected {redo} times: {reason}"),
            );
            info!(track = %job.track, name, shot, "generated video rejected");
            return self.close(job).await;
        }
        job.state.edit = Some(edit.to_owned());
        job.stage = drawn(look);
        self.send(job).await
    }

    pub(super) async fn send(&self, mut job: Gen) -> JobResult {
        let expected = job.step;
        job.step += 1;
        let outgoing = match job.stage {
            Stage::Keyframe => lanes::keyframe(&job, &self.config)?,
            Stage::Shot => lanes::shot(&job, &self.config, &self.storage_url)?,
            Stage::Assemble => lanes::assemble(&job, &self.config, &self.storage_url)?,
            stage => {
                return Err(JobError::permanent(anyhow::anyhow!(
                    "stage {} sends nothing",
                    stage.as_str()
                )));
            }
        };
        let sending = Save {
            sent: true,
            wait: Duration::ZERO,
        };
        if !state::save(&self.pool, &job, expected, sending).await? {
            return Ok(());
        }
        let published = self
            .lanes
            .publish(outgoing.subject, outgoing.payload, outgoing.message_id)
            .await;
        if let Err(error) = published {
            warn!(track = %job.track, %error, "video generation request was not published");
            let retry = Save {
                sent: false,
                wait: RETRY_PUBLISH,
            };
            state::save(&self.pool, &job, job.step, retry).await?;
        }
        Ok(())
    }

    pub(super) async fn fault(&self, mut job: Gen, reason: &str) -> JobResult {
        job.faults += 1;
        if job.faults >= self.config.max_faults {
            job.end(Stage::Failed, reason);
            warn!(track = %job.track, reason, "video generation failed");
            return self.close(job).await;
        }
        warn!(track = %job.track, faults = job.faults, reason, "video generation step is retried");
        let wait = self.config.retry_base * u32::try_from(job.faults).unwrap_or(1);
        state::save(&self.pool, &job, job.step, Save { sent: false, wait }).await?;
        Ok(())
    }

    pub(super) async fn close(&self, job: Gen) -> JobResult {
        let done = Save {
            sent: false,
            wait: Duration::ZERO,
        };
        if state::save(&self.pool, &job, job.step, done).await? {
            self.drop_temps(&job.state.temps).await;
        }
        Ok(())
    }

    pub(super) async fn drop_temps(&self, keys: &[String]) {
        for key in keys {
            if let Err(error) = self.store.delete(key).await {
                warn!(key, %error, "a generation temp file was not deleted; the sweep removes it");
            }
        }
    }

    async fn directed<T>(&self, job: &Gen, answer: Answer<T>) -> JobResult<Directed<T>> {
        let period = ledger::period(chrono::Utc::now(), self.reset_day);
        ledger::record(&self.pool, LADDER, period, &job.track, &answer.spend).await?;
        match answer.result {
            Ok(value) => {
                ledger::resume(&self.pool, LADDER).await?;
                Ok(Directed::Done(value))
            }
            Err(LlmError::Down(reason)) => {
                let text = format!(
                    "Video generation is paused: the Opus API is unreachable ({reason}). It checks every {} min and resumes by itself.",
                    self.config.recheck.as_secs() / 60
                );
                ledger::pause(
                    &self.pool,
                    &self.notify,
                    LADDER,
                    ledger::API_DOWN,
                    self.config.recheck,
                    &text,
                )
                .await?;
                Ok(Directed::Halted)
            }
            Err(LlmError::Invalid(reason)) => Ok(Directed::Invalid(reason)),
        }
    }

    async fn brief(&self, track: &str) -> JobResult<Option<Brief>> {
        let row = sqlx::query_file!("queries/video_gen/brief.sql", track)
            .fetch_optional(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        Ok(row.map(|row| Brief {
            track: track.to_owned(),
            title: row.title,
            artist: row.artist,
            duration_ms: row.duration_ms,
            cover_url: row.artwork_url.replace("-large.", "-t500x500."),
            lyrics: row.lyrics,
            shots: self.config.shots,
            shot_ms: self.config.shot_ms(),
        }))
    }

    async fn pictures(&self, keys: &[String], name: &str) -> JobResult<Option<Vec<Picture>>> {
        let mut pictures = Vec::with_capacity(keys.len());
        for (index, key) in keys.iter().enumerate() {
            let Some(data) = self.store.get(key).await.map_err(JobError::retryable)? else {
                return Ok(None);
            };
            let media_type = match key.ends_with(".png") {
                true => "image/png",
                false => "image/jpeg",
            };
            pictures.push(Picture {
                name: format!("{name}{index}"),
                media_type,
                data,
            });
        }
        Ok(Some(pictures))
    }
}

fn drawn(look: Look) -> Stage {
    match look {
        Look::Keyframe => Stage::Keyframe,
        Look::Shot => Stage::Shot,
    }
}
