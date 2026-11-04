use rand::Rng;
use rand::seq::SliceRandom;

use crate::modules::chat::model::DEVICES_MAX;
use crate::modules::chat::synthetic::{Seed, USER_PREFIX};
use crate::modules::chat::{ChatError, ChatResult};

pub const USERS_MAX: u32 = 50_000;
pub const SEEDED_DEVICES_MAX: u32 = 50_000;
pub const GROUP_MIN: usize = 2;
pub const GROUP_MAX: usize = 50;

pub fn check(seed: &Seed) -> ChatResult<()> {
    let users = seed.users as usize;
    let groups = seed.groups as usize;
    let devices = u64::from(seed.users) * u64::from(seed.devices_per_user);
    let fits = (GROUP_MIN as u32..=USERS_MAX).contains(&seed.users)
        && (1..=DEVICES_MAX as u32).contains(&seed.devices_per_user)
        && devices <= u64::from(SEEDED_DEVICES_MAX)
        && groups >= 1
        && groups * GROUP_MIN <= users
        && users <= groups * GROUP_MAX;
    if !fits {
        return Err(ChatError::Invalid);
    }
    Ok(())
}

pub fn user_id(index: u64) -> String {
    format!("{USER_PREFIX}{index}")
}

pub fn group_sizes(users: usize, groups: usize, rng: &mut impl Rng) -> Vec<usize> {
    let span = (GROUP_MAX - GROUP_MIN) as f64;
    let mean = users as f64 / groups as f64;
    let skew = (span / (mean - GROUP_MIN as f64 + 0.5)).max(1.0) - 1.0;
    let mut sizes: Vec<usize> = (0..groups)
        .map(|_| GROUP_MIN + (span * rng.gen_range(0.0..1.0f64).powf(skew)) as usize)
        .collect();
    settle(&mut sizes, users, rng);
    sizes
}

fn settle(sizes: &mut [usize], users: usize, rng: &mut impl Rng) {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.shuffle(rng);
    let mut total: usize = sizes.iter().sum();
    let mut floor = GROUP_MIN + 1;
    while total != users {
        let mut moved = false;
        for &at in &order {
            let size = &mut sizes[at];
            if total < users && *size >= floor && *size < GROUP_MAX {
                *size += 1;
                total += 1;
                moved = true;
            } else if total > users && *size > floor {
                *size -= 1;
                total -= 1;
                moved = true;
            }
            if total == users {
                return;
            }
        }
        if !moved {
            floor = GROUP_MIN;
        }
    }
}

pub fn split<'a>(users: &'a [String], sizes: &[usize]) -> Vec<&'a [String]> {
    let mut rest = users;
    sizes
        .iter()
        .map(|size| {
            let (head, tail) = rest.split_at(*size);
            rest = tail;
            head
        })
        .collect()
}
