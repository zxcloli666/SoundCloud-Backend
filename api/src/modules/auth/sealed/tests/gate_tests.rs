use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

use sqlx::PgPool;

use super::support::{Device, cfg, sealed, solve};
use crate::modules::auth::sealed::decide::start;
use crate::modules::auth::sealed::effort::Effort;
use crate::modules::auth::sealed::gate::{Refused, StartGate};
use crate::modules::auth::sealed::refusal::Refusal;
use crate::modules::auth::sealed::seed::{Seeds, epoch_now};
use crate::modules::chat::ChatError;

const LOGIN: &str = "/auth/login";

fn roomy() -> StartGate {
    let mut config = cfg();
    config.free_per_min = 1_000_000;
    config.pow_per_min = 1_000_000;
    StartGate::new(LOGIN, &config, 1_000_000)
}

fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

fn v6(network: u16, subnet: u16) -> IpAddr {
    IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, network, subnet, 0, 0, 0, 1))
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_flood_without_pow_from_65k_networks_fills_only_the_free_lane(
    pool: PgPool,
) -> anyhow::Result<()> {
    let node = sealed(cfg(), &pool)?;
    let gate = &node.gates.login;
    let (mut admitted, mut busy) = (0, 0);

    for start_number in 0..100_000u32 {
        let client = v6((start_number % 65_000) as u16, 0);
        match start(&node, gate, LOGIN, None, &[], client).await {
            Ok(()) => admitted += 1,
            Err(Refusal::Chat(ChatError::Busy { .. })) => busy += 1,
            Err(_) => panic!("a start without pow gets admitted or busy"),
        }
    }
    assert_eq!((admitted, busy), (120, 99_880));
    assert!(gate.required(Instant::now()) >= 1);

    let device = Device::new();
    let pow = solve(&node.seeds.seed(epoch_now()), 1, &device.auth_pub(), LOGIN);
    let passed = start(
        &node,
        gate,
        LOGIN,
        Some(&pow),
        &device.auth_pub(),
        v6(65_001, 0),
    )
    .await;
    assert!(passed.is_ok());
    Ok(())
}

#[test]
fn a_v4_24_is_capped_at_120_and_a_v6_48_at_60_in_both_lanes() {
    let now = Instant::now();
    let gate = roomy();
    let free = (1..=200).filter(|&host| gate.admit_free(v4(10, 1, 1, host), now).is_ok());
    assert_eq!(free.count(), 120);
    assert!(gate.admit_free(v4(10, 1, 2, 1), now).is_ok());

    let pow = (1..=200).filter(|&host| {
        let at = now + Duration::from_millis(u64::from(host));
        gate.admit_pow(v4(10, 2, 2, host), at).is_ok()
    });
    assert_eq!(pow.count(), 120);

    let mapped = IpAddr::V6(Ipv4Addr::new(10, 3, 3, 1).to_ipv6_mapped());
    assert!(gate.admit_free(mapped, now).is_ok());
    let rest = (2..=200).filter(|&host| gate.admit_free(v4(10, 3, 3, host), now).is_ok());
    assert_eq!(rest.count(), 119);

    let free = (0..200).filter(|&subnet| gate.admit_free(v6(7, subnet), now).is_ok());
    assert_eq!(free.count(), 60);
    assert!(gate.admit_free(v6(8, 0), now).is_ok());
}

#[test]
fn a_shared_network_cap_is_not_a_reason_for_pow() {
    let now = Instant::now();
    let gate = roomy();
    for host in 1..=120 {
        assert!(gate.admit_free(v4(10, 4, 4, host), now).is_ok());
    }
    assert!(matches!(
        gate.admit_free(v4(10, 4, 4, 200), now),
        Err(Refused::Capped(_))
    ));
}

#[test]
fn a_client_gets_15_free_logins_then_the_lane_refusal() {
    let now = Instant::now();
    let config = cfg();
    let gate = StartGate::new(LOGIN, &config, 15);
    let client = v4(192, 0, 2, 1);
    for _ in 0..15 {
        assert!(gate.admit_free(client, now).is_ok());
    }
    assert!(matches!(
        gate.admit_free(client, now),
        Err(Refused::Lane(_))
    ));
    assert!(gate.admit_free(v4(192, 0, 2, 2), now).is_ok());
}

#[test]
fn the_pow_lane_is_a_token_bucket_with_a_burst_of_50() {
    let mut config = cfg();
    config.pow_per_min = 600;
    let gate = StartGate::new(LOGIN, &config, 15);
    let now = Instant::now();
    for subnet in 0..50 {
        assert!(gate.admit_pow(v4(10, 9, subnet, 1), now).is_ok());
    }
    assert!(matches!(
        gate.admit_pow(v4(10, 9, 100, 1), now),
        Err(Refused::Lane(_))
    ));
    let later = now + Duration::from_secs(1);
    let refilled =
        (101..=130).filter(|&subnet| gate.admit_pow(v4(10, 9, subnet, 1), later).is_ok());
    assert_eq!(refilled.count(), 10);
}

#[test]
fn a_client_is_capped_at_60_pow_starts_a_minute() {
    let now = Instant::now();
    let gate = roomy();
    let client = v4(192, 0, 2, 50);
    let passed = (0..100u64).filter(|&step| {
        gate.admit_pow(client, now + Duration::from_millis(step))
            .is_ok()
    });
    assert_eq!(passed.count(), 60);
    assert!(matches!(
        gate.admit_pow(client, now + Duration::from_millis(100)),
        Err(Refused::Capped(_))
    ));
}

#[test]
fn the_shown_effort_is_zero_while_the_free_lane_has_room() {
    let now = Instant::now();
    let mut config = cfg();
    config.free_per_min = 2;
    config.pow_floor = 100;
    let gate = StartGate::new(LOGIN, &config, 15);
    assert_eq!(gate.required(now), 0);
    assert!(gate.admit_free(v4(192, 0, 2, 1), now).is_ok());
    assert_eq!(gate.required(now), 0);
    assert!(gate.admit_free(v4(192, 0, 2, 2), now).is_ok());
    assert_eq!(gate.required(now), 100);
    assert_eq!(gate.required(now + Duration::from_secs(61)), 0);
}

#[test]
fn the_effort_doubles_under_pow_load_up_to_the_cap_and_halves_when_it_passes() {
    let effort = Effort::new(100, 6000, 1000);
    assert_eq!((effort.current(), effort.minimum()), (100, 100));

    for _ in 0..600 {
        effort.note();
    }
    let mut levels = Vec::new();
    for _ in 0..6 {
        effort.tick();
        levels.push(effort.current());
    }
    assert_eq!(levels, [200, 400, 800, 1600, 3200, 6000]);
    assert_eq!(effort.minimum(), 3000);

    let mut falling = Vec::new();
    for _ in 0..7 {
        effort.tick();
        falling.push(effort.current());
    }
    assert_eq!(falling, [3000, 1500, 750, 375, 187, 100, 100]);
}

#[test]
fn a_light_load_never_raises_the_effort_over_the_floor() {
    let effort = Effort::new(100, 6000, 1000);
    for _ in 0..40 {
        for _ in 0..50 {
            effort.note();
        }
        effort.tick();
    }
    assert_eq!(effort.current(), 100);
}

#[test]
fn a_seed_lasts_two_epochs_and_belongs_to_one_node() {
    let seeds = Seeds::new([5; 32], "main".to_owned());
    let star = Seeds::new([5; 32], "star".to_owned());
    let other_key = Seeds::new([6; 32], "main".to_owned());
    let epoch = 1_000;
    assert!(seeds.accepts(&seeds.seed(epoch), epoch));
    assert!(seeds.accepts(&seeds.seed(epoch - 1), epoch));
    assert!(!seeds.accepts(&seeds.seed(epoch - 2), epoch));
    assert!(!seeds.accepts(&seeds.seed(epoch + 1), epoch));
    assert!(!seeds.accepts(&star.seed(epoch), epoch));
    assert!(!seeds.accepts(&other_key.seed(epoch), epoch));
    assert!(!seeds.accepts(&seeds.seed(epoch)[..31], epoch));
}

#[test]
fn the_effort_ticks_every_ten_seconds_of_traffic() {
    let mut config = cfg();
    config.pow_floor = 100;
    config.pow_cap = 6000;
    let gate = StartGate::new(LOGIN, &config, 15);
    for _ in 0..600 {
        gate.effort.note();
    }
    let now = Instant::now();
    gate.catch_up(now + Duration::from_secs(5));
    assert_eq!(gate.effort.current(), 100);
    gate.catch_up(now + Duration::from_secs(25));
    assert_eq!(gate.effort.current(), 400);
}
