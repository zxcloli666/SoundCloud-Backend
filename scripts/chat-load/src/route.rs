use std::collections::BTreeMap;

use anyhow::{Context, bail};
use chat_wire::proto::Tier;

pub const DEFAULT_ORIGIN: &str = "https://api.scnative.dev";
pub const DEFAULT_RELAYS: [(&str, &str); 2] = [
    ("r1", "https://api.r1.relay.scnative.dev"),
    ("r2", "https://api.r2.relay.scnative.dev"),
];
pub const DEFAULT_PROS: [(&str, &str); 1] = [("p1", "https://p1.pro.scnative.dev")];
pub const DEFAULT_MIX: &str = "direct=25,r1=25,r2=40,p1=10";
pub const DEFAULT_RELAY_CAP: usize = 9_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Direct,
    Relay,
    Pro,
}

impl Kind {
    pub fn tier(self) -> Tier {
        match self {
            Kind::Direct => Tier::Direct,
            Kind::Relay => Tier::Relay,
            Kind::Pro => Tier::Pro,
        }
    }

    pub fn answer_bytes(self) -> u32 {
        match self {
            Kind::Pro => 24 * 1024,
            Kind::Direct | Kind::Relay => 256 * 1024,
        }
    }

    pub fn hold_cap_ms(self) -> u64 {
        match self {
            Kind::Pro => 15_000,
            Kind::Direct | Kind::Relay => 20_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Route {
    pub name: String,
    pub kind: Kind,
    pub base: String,
}

#[derive(Debug, Clone)]
pub struct Routes {
    pub list: Vec<Route>,
    pub origin: String,
}

impl Routes {
    pub fn build(
        direct: Option<&str>,
        relays: &[(String, String)],
        pros: &[(String, String)],
        origin: Option<&str>,
    ) -> Self {
        let origin = trimmed(origin.unwrap_or(DEFAULT_ORIGIN));
        let mut list = vec![Route {
            name: "direct".to_owned(),
            kind: Kind::Direct,
            base: trimmed(direct.unwrap_or(&origin)),
        }];
        let defaults = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(name, url)| ((*name).to_owned(), (*url).to_owned()))
                .collect()
        };
        let relays = match relays.is_empty() {
            true => defaults(&DEFAULT_RELAYS),
            false => relays.to_vec(),
        };
        let pros = match pros.is_empty() {
            true => defaults(&DEFAULT_PROS),
            false => pros.to_vec(),
        };
        for (kind, pairs) in [(Kind::Relay, relays), (Kind::Pro, pros)] {
            for (name, url) in pairs {
                list.push(Route {
                    name,
                    kind,
                    base: trimmed(&url),
                });
            }
        }
        Self { list, origin }
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.list.iter().position(|route| route.name == name)
    }
}

pub fn named_url(raw: &str) -> anyhow::Result<(String, String)> {
    let (name, url) = raw
        .split_once('=')
        .with_context(|| format!("{raw} is not name=url"))?;
    if name.is_empty() || name == "direct" || !url.starts_with("http") {
        bail!("{raw} is not name=url");
    }
    Ok((name.to_owned(), url.to_owned()))
}

pub fn parse_mix(raw: &str, routes: &Routes) -> anyhow::Result<Vec<(usize, u32)>> {
    let mut mix = Vec::new();
    for part in raw
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (name, weight) = part
            .split_once('=')
            .with_context(|| format!("mix part {part} is not name=weight"))?;
        let route = routes
            .index_of(name)
            .with_context(|| format!("mix names {name}, which is not a route"))?;
        let weight: u32 = weight
            .parse()
            .with_context(|| format!("mix weight {weight} is not a number"))?;
        if weight > 0 {
            mix.push((route, weight));
        }
    }
    if mix.is_empty() {
        bail!("the mix gives no device a route");
    }
    Ok(mix)
}

pub fn assign(mix: &[(usize, u32)], count: usize) -> Vec<usize> {
    let total: f64 = mix.iter().map(|(_, weight)| f64::from(*weight)).sum();
    let mut given = vec![0usize; mix.len()];
    (0..count)
        .map(|at| {
            let wanted = |slot: usize| f64::from(mix[slot].1) / total * (at + 1) as f64;
            let slot = (0..mix.len())
                .max_by(|a, b| {
                    let behind_a = wanted(*a) - given[*a] as f64;
                    let behind_b = wanted(*b) - given[*b] as f64;
                    behind_a.total_cmp(&behind_b).then(b.cmp(a))
                })
                .unwrap_or(0);
            given[slot] += 1;
            mix[slot].0
        })
        .collect()
}

pub fn per_route(assignment: &[usize], routes: &Routes) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for route in assignment {
        *counts.entry(routes.list[*route].name.clone()).or_insert(0) += 1;
    }
    counts
}

pub fn check_relay_cap(assignment: &[usize], routes: &Routes, cap: usize) -> anyhow::Result<()> {
    for (name, count) in per_route(assignment, routes) {
        let relay = routes
            .index_of(&name)
            .is_some_and(|at| routes.list[at].kind == Kind::Relay);
        if relay && count > cap {
            bail!(
                "{count} devices on relay {name} is over the relay cap {cap}: every relay client holds two haproxy connections and r1 maxconn is 24000"
            );
        }
    }
    Ok(())
}

fn trimmed(url: &str) -> String {
    url.trim_end_matches('/').to_owned()
}

#[cfg(test)]
#[path = "tests/route_tests.rs"]
mod tests;
