use std::collections::{HashMap, HashSet};

use futures::{StreamExt, future, stream};
use uuid::Uuid;

use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::{ChatResult, ChatState};

pub const CLAIM: i64 = 500;
pub const CLAIMS: u32 = 4;
pub const PARALLEL: usize = 16;
pub const STUCK_ATTEMPTS: i16 = 10;

#[derive(Debug, Clone)]
struct Object {
    root: Uuid,
    store: i16,
    key: String,
}

#[derive(Debug, Default)]
struct Outcome {
    done: Vec<Uuid>,
    untried: Vec<Uuid>,
    stalled: bool,
}

pub async fn run(chat: &ChatState, budget: &mut Budget) -> ChatResult<u64> {
    let mut removed = 0;
    while budget.spend(chat) {
        let objects = claim(chat).await?;
        let roots = objects
            .iter()
            .map(|object| object.root)
            .collect::<HashSet<_>>()
            .len();
        if roots == 0 {
            break;
        }
        let outcome = delete_objects(chat, &objects, budget).await;
        if !outcome.untried.is_empty() {
            sqlx::query_file!("queries/chat/sweeps/media_unclaim.sql", &outcome.untried)
                .execute(&chat.pg)
                .await?;
        }
        if !outcome.done.is_empty() {
            sqlx::query_file!("queries/chat/sweeps/media_drop_done.sql", &outcome.done)
                .execute(&chat.pg)
                .await?;
        }
        removed += outcome.done.len() as u64;
        if outcome.stalled || (roots as i64) < CLAIM {
            break;
        }
    }
    Ok(removed)
}

async fn claim(chat: &ChatState) -> ChatResult<Vec<Object>> {
    let mut objects: Vec<Object> = sqlx::query_file!("queries/chat/sweeps/media_claim.sql", CLAIM)
        .fetch_all(&chat.pg)
        .await?
        .into_iter()
        .map(|row| Object {
            root: row.root,
            store: row.store,
            key: row.object_key,
        })
        .collect();
    objects.sort_by_key(|object| object.root);
    Ok(objects)
}

async fn delete_objects(chat: &ChatState, objects: &[Object], budget: &Budget) -> Outcome {
    let mut left: HashMap<Uuid, usize> = HashMap::new();
    for object in objects {
        *left.entry(object.root).or_default() += 1;
    }
    let mut failed: HashSet<Uuid> = HashSet::new();
    let mut failures_in_a_row = 0;
    let mut stalled = false;
    let mut results = stream::iter(objects.to_vec())
        .take_while(|_| future::ready(budget.in_time(chat)))
        .map(|object| delete_one(chat, object))
        .buffer_unordered(PARALLEL);
    while let Some((root, deleted)) = results.next().await {
        if deleted {
            failures_in_a_row = 0;
            if let Some(count) = left.get_mut(&root) {
                *count -= 1;
            }
            continue;
        }
        failed.insert(root);
        failures_in_a_row += 1;
        if failures_in_a_row >= PARALLEL {
            stalled = true;
            break;
        }
    }
    drop(results);
    let mut outcome = Outcome {
        stalled,
        ..Outcome::default()
    };
    for (root, count) in left {
        if failed.contains(&root) {
            continue;
        }
        if count == 0 {
            outcome.done.push(root);
        } else {
            outcome.untried.push(root);
        }
    }
    outcome
}

async fn delete_one(chat: &ChatState, object: Object) -> (Uuid, bool) {
    let Ok(store) = u16::try_from(object.store) else {
        return (object.root, false);
    };
    (
        object.root,
        chat.s3.delete(store, &object.key).await.is_ok(),
    )
}
