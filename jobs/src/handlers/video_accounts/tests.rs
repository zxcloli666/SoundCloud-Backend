use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use backend_contracts::admin_video::{AdminVideoCommand, VideoSource};
use sqlx::PgPool;

use super::check::mock::{Answer, Mock};
use super::{Checker, Notify, SpotifyKey, VideoAccounts, rpc};

const STATUS_URL: &str = "https://api.music.yandex.net/account/status";
const ALIVE: &str = r#"{"result":{"account":{"uid":1}}}"#;

#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<String>>,
    failing: AtomicBool,
}

impl Notify for Arc<Recorder> {
    async fn send(&self, text: &str) -> Result<(), String> {
        if self.failing.load(Ordering::SeqCst) {
            return Err("down".to_owned());
        }
        if let Ok(mut sent) = self.sent.lock() {
            sent.push(text.to_owned());
        }
        Ok(())
    }
}

impl Recorder {
    fn messages(&self) -> Vec<String> {
        self.sent
            .lock()
            .map(|sent| sent.clone())
            .unwrap_or_default()
    }
}

type Accounts = VideoAccounts<Arc<Mock>, Arc<Recorder>>;

fn accounts(pool: PgPool) -> (Arc<Accounts>, Arc<Mock>, Arc<Recorder>) {
    let mock = Mock::new();
    let recorder = Arc::new(Recorder::default());
    let checker = Checker::new(mock.clone(), SpotifyKey::new(61, &[1, 2, 3]));
    let accounts = VideoAccounts::new(pool, checker, recorder.clone(), Duration::from_secs(86_400));
    (Arc::new(accounts), mock, recorder)
}

async fn save(accounts: &Accounts, name: &str, secret: &str) -> anyhow::Result<u16> {
    let command = AdminVideoCommand::Save {
        source: VideoSource::Yandex,
        name: name.to_owned(),
        secret: secret.to_owned(),
    };
    Ok(rpc::handle(accounts, command).await?.status)
}

async fn status(pool: &PgPool, name: &str) -> anyhow::Result<String> {
    Ok(
        sqlx::query_scalar("SELECT status FROM video_accounts WHERE name = $1")
            .bind(name)
            .fetch_one(pool)
            .await?,
    )
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_dead_account_sends_one_message_and_stays_quiet_while_it_is_dead(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (accounts, mock, recorder) = accounts(pool.clone());
    mock.route(STATUS_URL, Answer::Body(ALIVE.to_owned()));
    assert_eq!(save(&accounts, "yandex1", "tok").await?, 200);

    mock.route(STATUS_URL, Answer::Status(401));
    accounts.check_account("yandex1").await?;
    accounts.check_account("yandex1").await?;
    accounts.check_account("yandex1").await?;

    assert_eq!(status(&pool, "yandex1").await?, "dead");
    let messages = recorder.messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("yandex1"));
    assert!(messages[0].contains("Яндекс Музыка"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_revived_account_returns_silently_and_a_second_death_is_announced(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (accounts, mock, recorder) = accounts(pool.clone());
    mock.route(STATUS_URL, Answer::Body(ALIVE.to_owned()));
    save(&accounts, "yandex1", "tok").await?;
    mock.route(STATUS_URL, Answer::Status(401));
    accounts.check_account("yandex1").await?;

    mock.route(STATUS_URL, Answer::Body(ALIVE.to_owned()));
    accounts.check_account("yandex1").await?;
    assert_eq!(status(&pool, "yandex1").await?, "ok");
    assert_eq!(recorder.messages().len(), 1);

    mock.route(STATUS_URL, Answer::Status(401));
    accounts.check_account("yandex1").await?;
    assert_eq!(recorder.messages().len(), 2);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_unreachable_service_changes_no_status_and_sends_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (accounts, mock, recorder) = accounts(pool.clone());
    mock.route(STATUS_URL, Answer::Body(ALIVE.to_owned()));
    save(&accounts, "yandex1", "tok").await?;

    mock.route(STATUS_URL, Answer::Down);
    accounts.check_account("yandex1").await?;

    assert_eq!(status(&pool, "yandex1").await?, "ok");
    assert!(recorder.messages().is_empty());
    let error: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM video_accounts WHERE name = 'yandex1'")
            .fetch_one(&pool)
            .await?;
    assert!(error.is_some());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_failed_telegram_send_is_retried_and_then_sent_once(pool: PgPool) -> anyhow::Result<()> {
    let (accounts, mock, recorder) = accounts(pool.clone());
    mock.route(STATUS_URL, Answer::Body(ALIVE.to_owned()));
    save(&accounts, "yandex1", "tok").await?;
    recorder.failing.store(true, Ordering::SeqCst);
    mock.route(STATUS_URL, Answer::Status(401));
    accounts.check_account("yandex1").await?;
    assert!(recorder.messages().is_empty());

    recorder.failing.store(false, Ordering::SeqCst);
    accounts.send_dead_alerts().await?;
    accounts.send_dead_alerts().await?;

    assert_eq!(recorder.messages().len(), 1);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_failing_secret_is_not_saved_and_the_error_comes_back(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (accounts, mock, _) = accounts(pool.clone());
    mock.route(STATUS_URL, Answer::Status(401));

    let status = save(&accounts, "yandex1", "bad").await?;

    assert_eq!(status, 422);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM video_accounts")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_list_shows_a_hash_prefix_and_never_the_secret(pool: PgPool) -> anyhow::Result<()> {
    let (accounts, mock, _) = accounts(pool.clone());
    mock.route(STATUS_URL, Answer::Body(ALIVE.to_owned()));
    save(&accounts, "yandex1", "super-secret-token").await?;

    let reply = rpc::handle(&accounts, AdminVideoCommand::List).await?;

    let body = reply.body.to_string();
    assert!(!body.contains("super-secret-token"));
    let prefix = &reply.body["accounts"][0]["secret_sha256"];
    assert_eq!(prefix.as_str().map(str::len), Some(12));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_daily_pass_checks_only_accounts_that_are_due(pool: PgPool) -> anyhow::Result<()> {
    let (accounts, mock, _) = accounts(pool.clone());
    mock.route(STATUS_URL, Answer::Body(ALIVE.to_owned()));
    save(&accounts, "yandex1", "tok").await?;
    sqlx::query(
        "INSERT INTO video_accounts (source, name, secret, last_checked_at)
         VALUES ('yandex', 'yandex2', 'old', now() - interval '2 days')",
    )
    .execute(&pool)
    .await?;
    let before = mock.requests().len();

    accounts
        .check_due(&tokio_util::sync::CancellationToken::new())
        .await?;

    assert_eq!(mock.requests().len() - before, 1);
    assert_eq!(status(&pool, "yandex2").await?, "ok");
    Ok(())
}
