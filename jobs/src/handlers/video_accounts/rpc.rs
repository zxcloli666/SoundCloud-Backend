use backend_contracts::admin_video::{AdminVideoCommand, AdminVideoReply, VideoSource};
use serde_json::{Value, json};

use super::{Fetch, Notify, Verdict, VideoAccounts, store};

const NAME_MAX: usize = 32;
const SECRET_MAX: usize = 64 * 1024;

pub(super) async fn handle<F: Fetch + 'static, N: Notify + 'static>(
    accounts: &VideoAccounts<F, N>,
    command: AdminVideoCommand,
) -> anyhow::Result<AdminVideoReply> {
    match command {
        AdminVideoCommand::List => list(accounts).await,
        AdminVideoCommand::Save {
            source,
            name,
            secret,
        } => save(accounts, source, &name, &secret).await,
        AdminVideoCommand::Check { name } => check(accounts, &name).await,
    }
}

fn reply(status: u16, body: Value) -> anyhow::Result<AdminVideoReply> {
    Ok(AdminVideoReply { status, body })
}

fn refusal(status: u16, message: &str) -> anyhow::Result<AdminVideoReply> {
    reply(status, json!({ "error": message }))
}

fn name_is_valid(name: &str) -> bool {
    (2..=NAME_MAX).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

async fn list<F: Fetch + 'static, N: Notify + 'static>(
    accounts: &VideoAccounts<F, N>,
) -> anyhow::Result<AdminVideoReply> {
    let rows = store::list(&accounts.pool, &accounts.seal).await?;
    reply(200, json!({ "accounts": rows }))
}

async fn save<F: Fetch + 'static, N: Notify + 'static>(
    accounts: &VideoAccounts<F, N>,
    source: VideoSource,
    name: &str,
    secret: &str,
) -> anyhow::Result<AdminVideoReply> {
    let secret = secret.trim();
    if !name_is_valid(name) {
        return refusal(400, "name: 2-32 chars, a-z 0-9 _ -");
    }
    if secret.is_empty() || secret.len() > SECRET_MAX {
        return refusal(400, "the secret is empty or too long");
    }
    match accounts.checker.check(source, secret).await {
        Verdict::Valid => {}
        Verdict::Rejected(error) => return refusal(422, &error),
        Verdict::Limited(error, _) | Verdict::Unavailable(error) => return refusal(503, &error),
    }
    if !store::upsert(
        &accounts.pool,
        &accounts.seal,
        source.as_str(),
        name,
        secret,
    )
    .await?
    {
        return refusal(409, "this name belongs to another source");
    }
    account(accounts, name, 200).await
}

async fn check<F: Fetch + 'static, N: Notify + 'static>(
    accounts: &VideoAccounts<F, N>,
    name: &str,
) -> anyhow::Result<AdminVideoReply> {
    if accounts.check_account(name).await?.is_none() {
        return refusal(404, "no such account");
    }
    account(accounts, name, 200).await
}

async fn account<F: Fetch + 'static, N: Notify + 'static>(
    accounts: &VideoAccounts<F, N>,
    name: &str,
    status: u16,
) -> anyhow::Result<AdminVideoReply> {
    let row = store::list(&accounts.pool, &accounts.seal)
        .await?
        .into_iter()
        .find(|row| row.name == name);
    reply(status, json!({ "account": row }))
}
