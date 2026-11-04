use std::sync::Arc;
use std::time::{Duration, Instant};

use backend_contracts::remote::{
    CommandKind, LinkProvider, RemoteCommand, RemoteReply, WaitResponse,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::hub::{Asked, Refused, RemoteHub, now_ms};
use crate::bus::nats::NatsService;

impl RemoteHub {
    async fn poll(
        &self,
        user: &str,
        device: &str,
        active: i64,
        since: Option<i64>,
        hold: Duration,
    ) -> WaitResponse {
        self.wait(user, device, active, since, hold)
            .await
            .expect("a wait slot")
    }

    async fn ask(&self, user: &str, source: &str, text: &str, wait: Duration) -> Asked {
        self.dispatch(
            user,
            CommandKind::Utterance,
            source,
            text,
            Instant::now() + wait,
        )
        .await
    }

    async fn confirm(&self, user: &str, provider: LinkProvider, hint: &str) -> RemoteCommand {
        let command = self
            .link_confirm(user, provider, hint)
            .await
            .expect("a reachable desktop");
        assert!(self.post(&command).await, "the confirmation was published");
        command
    }
}

fn prefix() -> String {
    format!("remote_test_{}", Uuid::now_v7().simple())
}

async fn host(prefix: &str) -> Arc<RemoteHub> {
    let url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned());
    let nats = NatsService::connect(&url, CancellationToken::new())
        .await
        .expect("nats");
    let hub = RemoteHub::new(Some(nats), prefix);
    assert!(hub.prepare().await, "the remote store opens");
    hub
}

async fn hosts<const N: usize>() -> [Arc<RemoteHub>; N] {
    let prefix = prefix();
    let mut opened = Vec::with_capacity(N);
    for _ in 0..N {
        opened.push(host(&prefix).await);
    }
    opened.try_into().unwrap_or_else(|_| unreachable!())
}

fn user() -> String {
    Uuid::now_v7().simple().to_string()
}

fn spoken(id: Uuid, text: &str) -> RemoteReply {
    RemoteReply {
        id,
        ok: true,
        speech: text.to_owned(),
        display: text.to_owned(),
        state: None,
    }
}

fn texts(response: &WaitResponse) -> Vec<&str> {
    response
        .commands
        .iter()
        .map(|command| command.text.as_str())
        .collect()
}

fn raw_command(user: &str, text: &str, created_at: i64) -> RemoteCommand {
    RemoteCommand {
        id: Uuid::now_v7(),
        kind: CommandKind::Utterance,
        seq: 0,
        user: user.to_owned(),
        source: "alice".to_owned(),
        text: text.to_owned(),
        hint: None,
        created_at,
        expires_at: now_ms() + 30_000,
    }
}

fn waiting(
    hub: &Arc<RemoteHub>,
    user: &str,
    device: &'static str,
    active: i64,
    cursor: i64,
    hold: Duration,
) -> tokio::task::JoinHandle<WaitResponse> {
    let hub = hub.clone();
    let user = user.to_owned();
    tokio::spawn(async move { hub.poll(&user, device, active, Some(cursor), hold).await })
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_command_from_one_host_reaches_a_desktop_on_another_and_the_reply_returns() {
    let [star, main] = hosts().await;
    let user = user();

    let handshake = main.poll(&user, "pc", 10, None, Duration::ZERO).await;
    let desktop = waiting(
        &main,
        &user,
        "pc",
        10,
        handshake.cursor,
        Duration::from_secs(10),
    );
    let asking = {
        let star = star.clone();
        let user = user.clone();
        tokio::spawn(async move {
            star.ask(&user, "alice", "следующий трек", Duration::from_secs(5))
                .await
        })
    };

    let delivered = desktop.await.expect("the waiter finishes");
    assert_eq!(texts(&delivered), ["следующий трек"]);
    let command = &delivered.commands[0];
    assert_eq!(command.source, "alice");
    assert!(command.seq > handshake.cursor);
    assert_eq!(delivered.cursor, command.seq);
    assert_eq!(delivered.active_device.as_deref(), Some("pc"));

    assert!(
        main.reply(&user, spoken(command.id, "Следующий трек"))
            .await
    );
    match asking.await.expect("the asker finishes") {
        Asked::Reply(reply) => assert_eq!(reply.speech, "Следующий трек"),
        _ => panic!("the reply did not come back to the asking host"),
    }
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn an_answer_survives_a_restart_of_the_asking_host() {
    let prefix = prefix();
    let star = host(&prefix).await;
    let main = host(&prefix).await;
    let user = user();

    let handshake = main.poll(&user, "pc", 1, None, Duration::ZERO).await;
    let command = star.confirm(&user, LinkProvider::Salut, "47").await;
    drop(star);
    let shown = main
        .poll(
            &user,
            "pc",
            1,
            Some(handshake.cursor),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(shown.commands[0].id, command.id);
    assert!(main.reply(&user, spoken(command.id, "")).await);

    let restarted = host(&prefix).await;
    let answer = restarted
        .answer(&user, command.id)
        .await
        .expect("the answer is read after the restart");
    assert!(answer.ok);
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn only_the_most_recently_active_desktop_gets_the_command() {
    let [first, second, star] = hosts().await;
    let user = user();

    let quiet = first.poll(&user, "laptop", 5, None, Duration::ZERO).await;
    let active = second.poll(&user, "desktop", 9, None, Duration::ZERO).await;
    assert_eq!(star.active_device(&user).await.as_deref(), Some("desktop"));
    let laptop = waiting(
        &first,
        &user,
        "laptop",
        5,
        quiet.cursor,
        Duration::from_secs(2),
    );
    let desktop = waiting(
        &second,
        &user,
        "desktop",
        9,
        active.cursor,
        Duration::from_secs(2),
    );

    let _ = star
        .ask(&user, "salut", "пауза", Duration::from_millis(300))
        .await;
    let laptop = laptop.await.expect("laptop");
    assert!(laptop.commands.is_empty());
    assert_eq!(laptop.active_device.as_deref(), Some("desktop"));
    assert_eq!(texts(&desktop.await.expect("desktop")), ["пауза"]);
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn asking_for_a_user_nobody_polls_for_stays_fast() {
    let [hub] = hosts().await;
    let started = Instant::now();
    let asked = hub
        .ask(&user(), "alice", "пауза", Duration::from_secs(3))
        .await;
    assert!(matches!(asked, Asked::Offline));
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn commands_from_two_hosts_between_two_waits_arrive_in_order() {
    let [first, second, main] = hosts().await;
    let user = user();
    let handshake = main.poll(&user, "pc", 1, None, Duration::ZERO).await;

    for (host, text) in [(&first, "раз"), (&second, "два"), (&first, "три")] {
        let _ = host
            .ask(&user, "alice", text, Duration::from_millis(50))
            .await;
    }

    let caught = main
        .poll(
            &user,
            "pc",
            1,
            Some(handshake.cursor),
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(texts(&caught), ["раз", "два", "три"]);
    let seqs: Vec<i64> = caught.commands.iter().map(|command| command.seq).collect();
    assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(caught.cursor, *seqs.last().expect("a cursor"));
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_command_sent_before_the_handshake_is_not_delivered() {
    let [star, main] = hosts().await;
    let user = user();
    main.poll(&user, "pc", 1, None, Duration::ZERO).await;
    let _ = star
        .ask(&user, "alice", "старая команда", Duration::from_millis(50))
        .await;

    let restarted = main.poll(&user, "pc", 1, None, Duration::ZERO).await;
    let quiet = main
        .poll(
            &user,
            "pc",
            1,
            Some(restarted.cursor),
            Duration::from_millis(200),
        )
        .await;
    assert!(quiet.commands.is_empty(), "{:?}", texts(&quiet));
    assert_eq!(quiet.cursor, restarted.cursor);
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn the_cursor_does_not_depend_on_the_clocks_of_the_hosts() {
    let [main] = hosts().await;
    let store = main.store().await.expect("a store");
    let user = user();
    let skew = 10 * 60 * 1000;

    store
        .publish(&raw_command(&user, "от спешащих часов", now_ms() + skew))
        .await
        .expect("published");
    let handshake = main.poll(&user, "pc", 1, None, Duration::ZERO).await;
    store
        .publish(&raw_command(&user, "от отстающих часов", now_ms() - skew))
        .await
        .expect("published");

    let caught = main
        .poll(
            &user,
            "pc",
            1,
            Some(handshake.cursor),
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(texts(&caught), ["от отстающих часов"]);
    assert!(caught.cursor > handshake.cursor);
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_cursor_from_before_a_stream_reset_gets_a_handshake_and_stays_fast() {
    let [main] = hosts().await;
    let user = user();
    let handshake = main.poll(&user, "pc", 1, None, Duration::ZERO).await;
    let started = Instant::now();
    let reset = main
        .poll(
            &user,
            "pc",
            1,
            Some(handshake.cursor + 1_000),
            Duration::from_secs(5),
        )
        .await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(reset.commands.is_empty());
    assert_eq!(reset.cursor, handshake.cursor);
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_link_confirmation_is_a_command_of_its_own_kind() {
    let [star, main] = hosts().await;
    let user = user();
    let handshake = main.poll(&user, "pc", 1, None, Duration::ZERO).await;
    let sent = star.confirm(&user, LinkProvider::Salut, "47").await;

    let delivered = main
        .poll(
            &user,
            "pc",
            1,
            Some(handshake.cursor),
            Duration::from_secs(1),
        )
        .await;
    let command = &delivered.commands[0];
    assert_eq!(command.id, sent.id);
    assert_eq!(command.kind, CommandKind::LinkConfirm);
    assert_eq!(command.source, "salut");
    assert_eq!(command.hint.as_deref(), Some("47"));
    assert!(command.expires_at - command.created_at > 55_000);

    assert!(star.answer(&user, command.id).await.is_none());
    assert!(main.reply(&user, spoken(command.id, "")).await);
    assert!(star.answer(&user, command.id).await.expect("an answer").ok);
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn an_answered_link_confirm_never_reaches_a_desktop_that_takes_over_on_another_host() {
    let [first, second] = hosts().await;
    let user = user();
    let laptop = second.poll(&user, "laptop", 1, None, Duration::ZERO).await;
    let desktop = first.poll(&user, "desktop", 5, None, Duration::ZERO).await;

    let sent = first.confirm(&user, LinkProvider::Alice, "47").await;
    let shown = first
        .poll(
            &user,
            "desktop",
            5,
            Some(desktop.cursor),
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(shown.commands[0].id, sent.id);
    assert!(first.reply(&user, spoken(sent.id, "")).await);

    let taken_over = second
        .poll(
            &user,
            "laptop",
            9,
            Some(laptop.cursor),
            Duration::from_millis(200),
        )
        .await;
    assert!(taken_over.commands.is_empty(), "{:?}", texts(&taken_over));
    assert_eq!(taken_over.active_device.as_deref(), Some("laptop"));
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn the_first_answer_wins() {
    let [star, main] = hosts().await;
    let user = user();
    main.poll(&user, "pc", 1, None, Duration::ZERO).await;
    let sent = star.confirm(&user, LinkProvider::Alice, "47").await;

    assert!(main.reply(&user, spoken(sent.id, "да")).await);
    let mut no = spoken(sent.id, "нет");
    no.ok = false;
    assert!(main.reply(&user, no).await);

    let answer = star.answer(&user, sent.id).await.expect("an answer");
    assert!(answer.ok);
    assert_eq!(answer.speech, "да");
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_reply_from_another_user_does_not_answer_the_command() {
    let [star, main] = hosts().await;
    let user = user();
    let stranger = self::user();
    let handshake = main.poll(&user, "pc", 1, None, Duration::ZERO).await;
    let asking = {
        let star = star.clone();
        let user = user.clone();
        tokio::spawn(async move {
            star.ask(&user, "alice", "пауза", Duration::from_millis(800))
                .await
        })
    };
    let shown = main
        .poll(
            &user,
            "pc",
            1,
            Some(handshake.cursor),
            Duration::from_secs(1),
        )
        .await;
    let id = shown.commands[0].id;

    assert!(main.reply(&stranger, spoken(id, "чужой ответ")).await);
    assert!(matches!(asking.await.expect("asker"), Asked::Silent));
    assert!(star.answer(&user, id).await.is_none());

    assert!(main.reply(&user, spoken(id, "свой ответ")).await);
    assert_eq!(
        star.answer(&user, id).await.expect("an answer").speech,
        "свой ответ"
    );
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_late_dispatch_ends_at_the_request_deadline_and_stays_fast() {
    let [hub] = hosts().await;
    let user = user();
    hub.poll(&user, "pc", 1, None, Duration::ZERO).await;
    let started = Instant::now() - Duration::from_millis(300);
    let _ = hub
        .dispatch(
            &user,
            CommandKind::Utterance,
            "alice",
            "x",
            started + Duration::from_millis(500),
        )
        .await;
    assert!(
        started.elapsed() < Duration::from_millis(700),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_polling_loop_writes_presence_once() {
    let [hub] = hosts().await;
    let store = hub.store().await.expect("a store");
    let user = user();
    hub.poll(&user, "pc", 7, None, Duration::ZERO).await;
    let first = store.device_revision(&user).await.expect("a roster entry");
    for _ in 0..50 {
        hub.poll(&user, "pc", 7, None, Duration::ZERO).await;
    }
    assert_eq!(store.device_revision(&user).await, Some(first));
    hub.poll(&user, "pc", 8, None, Duration::ZERO).await;
    assert!(store.device_revision(&user).await > Some(first));
}

#[tokio::test]
async fn presence_goes_out_only_for_a_new_device_a_new_activity_or_a_stale_entry() {
    let hub = RemoteHub::new(None, &prefix());
    let start = Instant::now();
    assert!(hub.presence_due("1", "pc", 5, start));
    assert!(!hub.presence_due("1", "pc", 5, start + Duration::from_secs(1)));
    assert!(hub.presence_due("1", "laptop", 5, start + Duration::from_secs(1)));
    assert!(hub.presence_due("1", "pc", 6, start + Duration::from_secs(2)));
    assert!(!hub.presence_due("1", "pc", 6, start + Duration::from_secs(3)));
    let stale = start + Duration::from_secs(2) + super::hub::PRESENCE_REFRESH;
    assert!(hub.presence_due("1", "pc", 6, stale));
}

#[tokio::test]
async fn without_nats_waits_are_refused_and_skills_see_no_desktop() {
    let hub = RemoteHub::new(None, &prefix());
    let refused = hub.wait("1", "pc", 1, None, Duration::ZERO).await;
    assert!(matches!(refused, Err(Refused::Unavailable)));
    assert!(matches!(
        hub.ask("1", "alice", "пауза", Duration::from_secs(1)).await,
        Asked::Offline
    ));
    assert!(!hub.reply("1", spoken(Uuid::now_v7(), "")).await);
}
