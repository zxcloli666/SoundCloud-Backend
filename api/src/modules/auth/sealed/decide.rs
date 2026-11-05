use std::net::IpAddr;
use std::time::Instant;

use sc_chat_wire::pow::verify;
use sc_chat_wire::proto::Pow;

use crate::common::admission::client_identity;
use crate::common::windows::Clients;
use crate::modules::auth::sealed::Sealed;
use crate::modules::auth::sealed::claims::{SOLUTION_TTL, claim, solution_key};
use crate::modules::auth::sealed::gate::{Refused, StartGate};
use crate::modules::auth::sealed::metrics::note;
use crate::modules::auth::sealed::refusal::Refusal;
use crate::modules::auth::sealed::seed::epoch_now;
use crate::modules::chat::ChatError;
use crate::modules::chat::errors::BusySource;

pub async fn start(
    sealed: &Sealed,
    gate: &StartGate,
    path: &'static str,
    pow: Option<&Pow>,
    auth_pub: &[u8],
    client: IpAddr,
) -> Result<(), Refusal> {
    let pow = pow.filter(|_| sealed.cfg.pow);
    let (lane, result) = match pow {
        Some(pow) if !pow.solution.is_empty() => (
            "pow",
            solved(sealed, gate, path, pow, auth_pub, client).await,
        ),
        marker => ("free", free(sealed, gate, marker.is_some(), client)),
    };
    note(path, lane, outcome(&result));
    result
}

pub fn poll(clients: &Clients, path: &'static str, client: IpAddr) -> Result<(), Refusal> {
    let result = clients
        .admit(&client_identity(client), Instant::now())
        .map_err(busy);
    note(path, "poll", outcome(&result));
    result
}

fn free(sealed: &Sealed, gate: &StartGate, capable: bool, client: IpAddr) -> Result<(), Refusal> {
    match gate.admit_free(client, Instant::now()) {
        Ok(()) => Ok(()),
        Err(Refused::Lane(_)) if capable => Err(required(sealed, gate)),
        Err(Refused::Lane(retry_ms) | Refused::Capped(retry_ms)) => Err(busy(retry_ms)),
    }
}

async fn solved(
    sealed: &Sealed,
    gate: &StartGate,
    path: &str,
    pow: &Pow,
    auth_pub: &[u8],
    client: IpAddr,
) -> Result<(), Refusal> {
    if !sealed.seeds.accepts(&pow.seed, epoch_now()) || pow.effort < gate.effort.minimum() {
        return Err(required(sealed, gate));
    }
    let started = Instant::now();
    let checked = verify(pow, auth_pub, path);
    metrics::histogram!("auth_pow_verify_seconds").record(started.elapsed().as_secs_f64());
    checked.map_err(|_| ChatError::Invalid)?;
    let key = solution_key(&pow.seed, &pow.nonce, auth_pub);
    claim(&sealed.redis, key, SOLUTION_TTL).await?;
    match gate.admit_pow(client, Instant::now()) {
        Ok(()) => Ok(()),
        Err(Refused::Lane(_)) => Err(required(sealed, gate)),
        Err(Refused::Capped(retry_ms)) => Err(busy(retry_ms)),
    }
}

fn required(sealed: &Sealed, gate: &StartGate) -> Refusal {
    Refusal::Required {
        seed: sealed.seeds.seed(epoch_now()),
        effort: gate.effort.current(),
    }
}

fn busy(retry_ms: u64) -> Refusal {
    Refusal::Chat(ChatError::Busy {
        retry_ms,
        source: BusySource::AuthGate,
    })
}

fn outcome(result: &Result<(), Refusal>) -> &'static str {
    match result {
        Ok(()) => "admitted",
        Err(refusal) => refusal.outcome(),
    }
}
