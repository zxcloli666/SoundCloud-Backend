use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, ensure};
use async_nats::jetstream::consumer::PriorityPolicy;
use async_nats::jetstream::stream::Stream;
use async_nats::{Client, Message, Subscriber};
use backend_contracts::worker_contract::{PriorityGroup, SPEAK_LANE, WorkerLaneSpec};
use futures::StreamExt;
use serde_json::{Value, json};
use uuid::Uuid;

use super::worker_consumers::ConsumerProvision;
use super::worker_tests::{leak, live_bus, scratch_stream};
use super::{Bus, pipeline_stream_config};

const PULL_EXPIRES_NANOS: u64 = 60_000_000_000;
const REPLY_WAIT: Duration = Duration::from_secs(5);
const LINES_WHILE_ONLY_WORKERS_WAIT: usize = 10;
const LINES_WHILE_BRIDGE_WAITS: usize = 20;

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn an_ungrouped_speak_consumer_gains_its_priority_group_in_place_and_then_refuses_a_pull_without_one()
-> anyhow::Result<()> {
    let bus = live_bus("speak-priority-update-test").await?;
    let lane = scratch_speak_lane();
    let group = speak_group()?;
    let stream = bus
        .ensure_stream(pipeline_stream_config(&lane.stream))
        .await?;
    let ungrouped = WorkerLaneSpec {
        priority_group: None,
        ..lane
    };
    assert_eq!(
        bus.ensure_worker_consumer(&ungrouped).await?,
        ConsumerProvision::Created
    );
    let mut worker = Puller::open(&bus.client, &lane, grouped_pull(group, group.workers)).await?;
    let mut legacy_worker = Puller::open(&bus.client, &lane, ungrouped_pull()).await?;

    publish_line(&bus, &lane).await?;
    worker.pull(&bus.client).await?;
    let line = worker.next_reply().await?;
    ensure!(
        line.status.is_none(),
        "a grouped pull on an ungrouped consumer got {:?} {:?}",
        line.status,
        line.description
    );
    ack(&bus.client, &line).await?;

    publish_line(&bus, &lane).await?;
    assert_eq!(
        bus.ensure_worker_consumer(&lane).await?,
        ConsumerProvision::Updated
    );
    assert_eq!(
        bus.ensure_worker_consumer(&lane).await?,
        ConsumerProvision::Unchanged
    );
    let info = stream.consumer_info(lane.durable).await?;
    assert_eq!(info.config.priority_groups, vec![group.name.to_owned()]);
    assert_eq!(info.config.priority_policy, PriorityPolicy::Prioritized);
    assert_eq!(
        info.num_pending, 1,
        "a line waiting on the lane must outlive the update"
    );

    legacy_worker.pull(&bus.client).await?;
    let refusal = legacy_worker.next_reply().await?;
    assert_eq!(refusal.status.map(|status| status.as_u16()), Some(400));
    assert_eq!(
        refusal.description.as_deref(),
        Some("Bad Request - Priority Group missing")
    );

    worker.pull(&bus.client).await?;
    let line = worker.next_reply().await?;
    assert!(line.status.is_none());
    ack(&bus.client, &line).await?;

    bus.jetstream.delete_stream(lane.stream.name).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_waiting_bridge_gets_every_line_and_the_workers_get_lines_only_while_it_does_not_wait()
-> anyhow::Result<()> {
    let bus = live_bus("speak-priority-test").await?;
    let lane = scratch_speak_lane();
    let group = speak_group()?;
    let stream = bus
        .ensure_stream(pipeline_stream_config(&lane.stream))
        .await?;
    assert_eq!(
        bus.ensure_worker_consumer(&lane).await?,
        ConsumerProvision::Created
    );
    let mut pullers = Pullers {
        bridge: Puller::open(&bus.client, &lane, grouped_pull(group, group.bridge)).await?,
        workers: Puller::open(&bus.client, &lane, grouped_pull(group, group.workers)).await?,
    };

    pullers.workers.pull(&bus.client).await?;
    let without_bridge = hand_out(
        &bus,
        &stream,
        &lane,
        &mut pullers,
        1,
        LINES_WHILE_ONLY_WORKERS_WAIT,
    )
    .await?;
    assert_eq!(
        without_bridge,
        vec![Taker::Workers; LINES_WHILE_ONLY_WORKERS_WAIT]
    );

    pullers.bridge.pull(&bus.client).await?;
    let with_bridge = hand_out(
        &bus,
        &stream,
        &lane,
        &mut pullers,
        2,
        LINES_WHILE_BRIDGE_WAITS,
    )
    .await?;
    assert_eq!(with_bridge, vec![Taker::Bridge; LINES_WHILE_BRIDGE_WAITS]);

    bus.jetstream.delete_stream(lane.stream.name).await?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Taker {
    Bridge,
    Workers,
}

struct Pullers {
    bridge: Puller,
    workers: Puller,
}

impl Pullers {
    async fn take_line(&mut self, client: &Client) -> anyhow::Result<Taker> {
        let taken = tokio::time::timeout(REPLY_WAIT, async {
            tokio::select! {
                Some(message) = self.bridge.replies.next() => Some((Taker::Bridge, message)),
                Some(message) = self.workers.replies.next() => Some((Taker::Workers, message)),
                else => None,
            }
        })
        .await
        .context("no line reached a puller")?;
        let (taker, line) = taken.ok_or_else(|| anyhow!("both pullers lost their inbox"))?;
        ensure!(
            line.status.is_none(),
            "{taker:?} got {:?} {:?} instead of a line",
            line.status,
            line.description
        );
        ack(client, &line).await?;
        match taker {
            Taker::Bridge => self.bridge.pull(client).await?,
            Taker::Workers => self.workers.pull(client).await?,
        }
        Ok(taker)
    }
}

struct Puller {
    inbox: String,
    replies: Subscriber,
    request: String,
    next_subject: String,
}

impl Puller {
    async fn open(client: &Client, lane: &WorkerLaneSpec, request: Value) -> anyhow::Result<Self> {
        let inbox = client.new_inbox();
        let replies = client.subscribe(inbox.clone()).await?;
        Ok(Self {
            inbox,
            replies,
            request: request.to_string(),
            next_subject: format!(
                "$JS.API.CONSUMER.MSG.NEXT.{}.{}",
                lane.stream.name, lane.durable
            ),
        })
    }

    async fn pull(&self, client: &Client) -> anyhow::Result<()> {
        client
            .publish_with_reply(
                self.next_subject.clone(),
                self.inbox.clone(),
                self.request.clone().into(),
            )
            .await?;
        client.flush().await?;
        Ok(())
    }

    async fn next_reply(&mut self) -> anyhow::Result<Message> {
        tokio::time::timeout(REPLY_WAIT, self.replies.next())
            .await
            .context("no reply reached the puller")?
            .context("the puller lost its inbox")
    }
}

async fn hand_out(
    bus: &Bus,
    stream: &Stream,
    lane: &WorkerLaneSpec,
    pullers: &mut Pullers,
    waiting_pulls: usize,
    lines: usize,
) -> anyhow::Result<Vec<Taker>> {
    let mut takers = Vec::with_capacity(lines);
    for _ in 0..lines {
        wait_for_waiting_pulls(stream, lane, waiting_pulls).await?;
        publish_line(bus, lane).await?;
        takers.push(pullers.take_line(&bus.client).await?);
    }
    Ok(takers)
}

async fn wait_for_waiting_pulls(
    stream: &Stream,
    lane: &WorkerLaneSpec,
    waiting_pulls: usize,
) -> anyhow::Result<()> {
    let started = Instant::now();
    loop {
        let info = stream.consumer_info(lane.durable).await?;
        if info.num_waiting == waiting_pulls && info.num_ack_pending == 0 {
            return Ok(());
        }
        ensure!(
            started.elapsed() < REPLY_WAIT,
            "the consumer kept {} waiting pulls and {} unacked lines",
            info.num_waiting,
            info.num_ack_pending
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn publish_line(bus: &Bus, lane: &WorkerLaneSpec) -> anyhow::Result<()> {
    bus.jetstream
        .publish(lane.filter_subject.to_owned(), "line".into())
        .await?
        .await?;
    Ok(())
}

async fn ack(client: &Client, line: &Message) -> anyhow::Result<()> {
    let reply = line
        .reply
        .clone()
        .ok_or_else(|| anyhow!("a line came without an ack subject"))?;
    client.publish(reply, "+ACK".into()).await?;
    client.flush().await?;
    Ok(())
}

fn grouped_pull(group: PriorityGroup, priority: u8) -> Value {
    json!({
        "batch": 1,
        "expires": PULL_EXPIRES_NANOS,
        "group": group.name,
        "priority": priority,
    })
}

fn ungrouped_pull() -> Value {
    json!({ "batch": 1, "expires": PULL_EXPIRES_NANOS })
}

fn speak_group() -> anyhow::Result<PriorityGroup> {
    SPEAK_LANE
        .priority_group
        .ok_or_else(|| anyhow!("the speak lane has no priority group"))
}

fn scratch_speak_lane() -> WorkerLaneSpec {
    let suffix = Uuid::now_v7().simple().to_string();
    WorkerLaneSpec {
        stream: scratch_stream("SPEAK_PRIORITY", &suffix),
        durable: leak(format!("speak-test-{suffix}")),
        filter_subject: leak(format!("test.speak_priority.{suffix}.say")),
        ..SPEAK_LANE
    }
}
