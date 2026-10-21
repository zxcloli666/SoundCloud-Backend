use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use tokio::process::Command;
use tokio::sync::Notify;
use tokio::time::{Instant, sleep, sleep_until};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::api::{Client, Clock};
use crate::cli::RunArgs;
use crate::device::{Behavior, Device, Mode, Shared};
use crate::layout::{Identity, Layout};
use crate::report::{self, Context};
use crate::route::{self, Routes};
use crate::schedule::Schedule;
use crate::stats::Stats;
use crate::watch::{self, Abort, Watcher};
use crate::wire::Ock;

pub const TICK: Duration = Duration::from_secs(1);
pub const STOP_GRACE: Duration = Duration::from_secs(3);
pub const SHARE_SCALE: f64 = 1_000.0;

pub struct Plan {
    pub identities: Vec<Arc<Identity>>,
    pub chats: Vec<Vec<Uuid>>,
    pub routes: Routes,
    pub schedule: Schedule,
    pub ock: Ock,
    pub order: Vec<usize>,
    pub assignment: Vec<usize>,
    pub parked: Vec<bool>,
    pub focused: Vec<bool>,
}

fn named(raw: &[String]) -> anyhow::Result<Vec<(String, String)>> {
    raw.iter().map(|one| route::named_url(one)).collect()
}

fn share(value: f64) -> u32 {
    (value.clamp(0.0, 1.0) * SHARE_SCALE).round() as u32
}

pub fn plan(args: &RunArgs) -> anyhow::Result<Plan> {
    let layout = Layout::load(&args.layouts)?;
    let routes = Routes::build(
        args.direct.as_deref(),
        &named(&args.relays)?,
        &named(&args.pros)?,
        args.origin.as_deref(),
    );
    let mix = route::parse_mix(&args.mix, &routes)?;
    let schedule = Schedule {
        steps: args.steps.clone(),
        step: Duration::from_secs(args.step_secs),
        spread: Duration::from_secs(args.spread_secs),
        hold: Duration::from_secs(args.hold_secs),
        restart_at: args
            .restart_cmd
            .as_ref()
            .map(|_| Duration::from_secs(args.restart_at_secs)),
        worst: Duration::from_secs(args.worst_secs),
    };
    schedule.check(layout.identities.len())?;
    let ock = match &args.ock_pub {
        Some(public) => Ock::from_hex(args.ock_version, public)?,
        None => Ock::pinned(args.ock_version)?,
    };
    let peak = schedule.peak();
    let mut rng = StdRng::from_entropy();
    let mut order: Vec<usize> = (0..layout.identities.len()).collect();
    order.shuffle(&mut rng);
    order.truncate(peak);
    let assignment = route::assign(&mix, peak);
    route::check_relay_cap(&assignment, &routes, args.relay_cap)?;
    let parked_weight = share(args.parked_share);
    let parked: Vec<bool> =
        route::assign(&[(1, parked_weight), (0, share(1.0) - parked_weight)], peak)
            .into_iter()
            .map(|slot| slot == 1)
            .collect();
    let focused = (0..peak)
        .map(|_| rng.gen_bool(args.focus_share.clamp(0.0, 1.0)))
        .collect();
    let chats = layout
        .identities
        .iter()
        .map(|identity| layout.chats_of(&identity.user))
        .collect();
    Ok(Plan {
        identities: layout.identities.into_iter().map(Arc::new).collect(),
        chats,
        routes,
        schedule,
        ock,
        order,
        assignment,
        parked,
        focused,
    })
}

fn every(seconds: u64) -> Option<Duration> {
    (seconds > 0).then(|| Duration::from_secs(seconds))
}

pub async fn run(args: RunArgs) -> anyhow::Result<bool> {
    let plan = plan(&args)?;
    let per_route = route::per_route(&plan.assignment, &plan.routes);
    let parked_count = plan.parked.iter().filter(|parked| **parked).count();
    println!(
        "plan: {} devices at peak, {} parked, routes {per_route:?}, {} s in all",
        plan.schedule.peak(),
        parked_count,
        plan.schedule.total().as_secs()
    );
    let music_baseline = match &args.music_metrics_cmd {
        Some(command) => {
            println!(
                "measuring the music p99 baseline for {} s",
                args.baseline_secs
            );
            let baseline = watch::baseline(command, Duration::from_secs(args.baseline_secs)).await;
            println!("music p99 baseline: {baseline:?} s");
            baseline
        }
        None => None,
    };
    let names: Vec<String> = plan
        .routes
        .list
        .iter()
        .map(|route| route.name.clone())
        .collect();
    let stats = Arc::new(Stats::new(&names));
    let start = Instant::now();
    let shared = Arc::new(Shared {
        stats: stats.clone(),
        behavior: Behavior {
            hold_ms: args.hold_ms,
            short_every: Duration::from_secs(args.short_every_secs.max(1)),
            send_every: every(args.send_every_secs),
            type_every: every(args.type_every_secs),
            run_id: rand::random(),
        },
        all_parked: AtomicBool::new(false),
        park_now: Notify::new(),
        clock: Arc::new(Clock::default()),
    });
    let abort = Arc::new(Abort::default());
    let stop = abort.token.clone();
    let watcher = Arc::new(Watcher {
        checks: args
            .watches
            .iter()
            .map(|raw| watch::named_check(raw))
            .collect::<anyhow::Result<_>>()?,
        every: Duration::from_secs(args.watch_every_secs.max(1)),
        error_rate: args.abort_error_rate,
        chat_metrics: args.chat_metrics_cmd.clone(),
        pool_wait_ms: args.abort_pool_wait_ms,
        music_metrics: args.music_metrics_cmd.clone(),
        music_rise: args.abort_music_rise,
        music_baseline,
    });

    tokio::spawn(ticker(stats.clone(), stop.clone()));
    tokio::spawn(status(
        stats.clone(),
        plan.schedule.clone(),
        start,
        Duration::from_secs(args.status_every_secs.max(1)),
        stop.clone(),
    ));
    tokio::spawn(watch::run(
        watcher,
        stats.clone(),
        abort.clone(),
        stop.clone(),
    ));
    tokio::spawn(interrupt(abort.clone(), stats.clone()));
    if let (Some(command), Some(at)) = (args.restart_cmd.clone(), plan.schedule.restart()) {
        tokio::spawn(restart(command, start + at, stats.clone(), stop.clone()));
    }
    if !plan.schedule.worst.is_zero() {
        tokio::spawn(worst_case(
            shared.clone(),
            start + plan.schedule.worst_starts(),
            stop.clone(),
        ));
    }
    let schedule = plan.schedule.clone();
    let launcher = tokio::spawn(launch(Arc::new(plan), shared.clone(), start, stop.clone()));

    tokio::select! {
        () = stop.cancelled() => {}
        () = sleep_until(start + schedule.total()) => {}
    }
    stop.cancel();
    launcher.abort();
    sleep(STOP_GRACE).await;
    stats.tick();

    let built = report::build(
        &stats,
        Context {
            run_id: shared.behavior.run_id,
            schedule,
            devices_per_route: per_route,
            parked_share: args.parked_share,
            abort: abort.reason(),
        },
    );
    std::fs::write(&args.report, serde_json::to_vec_pretty(&built)?)?;
    println!("{}", report::summary(&built));
    println!("report: {}", args.report.display());
    Ok(built.verdict.passed)
}

async fn launch(launch: Arc<Plan>, shared: Arc<Shared>, start: Instant, stop: CancellationToken) {
    for (position, identity_at) in launch.order.iter().enumerate() {
        let Some(at) = launch.schedule.launch_at(position) else {
            return;
        };
        tokio::select! {
            () = stop.cancelled() => return,
            () = sleep_until(start + at) => {}
        }
        let route_at = launch.assignment[position];
        let route = &launch.routes.list[route_at];
        let identity = launch.identities[*identity_at].clone();
        let client = match Client::new(
            identity.clone(),
            launch.ock,
            route,
            &launch.routes.origin,
            shared.clock.clone(),
        ) {
            Ok(client) => client,
            Err(error) => {
                eprintln!("device {} has no client: {error}", identity.index);
                continue;
            }
        };
        let device = Arc::new(Device::new(
            identity,
            route_at,
            client,
            launch.chats[*identity_at].clone(),
            Mode {
                parked: launch.parked[position],
                focused: launch.focused[position],
            },
            shared.clone(),
        ));
        tokio::spawn(device.run(stop.child_token()));
    }
}

async fn ticker(stats: Arc<Stats>, stop: CancellationToken) {
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            () = sleep(TICK) => {}
        }
        stats.tick();
    }
}

async fn status(
    stats: Arc<Stats>,
    schedule: Schedule,
    start: Instant,
    every: Duration,
    stop: CancellationToken,
) {
    let mut earlier = None;
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            () = sleep(every) => {}
        }
        let now = stats.snapshot();
        let phase = schedule.phase(start.elapsed());
        println!("{}", report::status_line(&stats, phase, earlier, now));
        earlier = Some(now);
    }
}

async fn interrupt(abort: Arc<Abort>, stats: Arc<Stats>) {
    if tokio::signal::ctrl_c().await.is_ok() {
        abort.trip(stats.elapsed_ms(), "interrupted".to_owned());
    }
}

async fn restart(command: String, at: Instant, stats: Arc<Stats>, stop: CancellationToken) {
    tokio::select! {
        () = stop.cancelled() => return,
        () = sleep_until(at) => {}
    }
    println!("[{:>5}s] restart: {command}", stats.elapsed_ms() / 1_000);
    stats.mark_restart();
    let ran = Command::new("sh").arg("-c").arg(&command).status().await;
    println!(
        "[{:>5}s] restart finished: {ran:?}",
        stats.elapsed_ms() / 1_000
    );
}

async fn worst_case(shared: Arc<Shared>, at: Instant, stop: CancellationToken) {
    tokio::select! {
        () = stop.cancelled() => return,
        () = sleep_until(at) => {}
    }
    println!(
        "[{:>5}s] worst case: every online device parks",
        shared.stats.elapsed_ms() / 1_000
    );
    shared.all_parked.store(true, Ordering::Relaxed);
    shared.park_now.notify_waiters();
}
