mod bench;
mod breaker;
mod budget;
mod charges;
mod providers;
mod races;
mod spend;
mod vendor;
mod worker;

use serde::Deserialize;

const ANSWER: &str = "{\"say\":\"hello there\"}";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoHostSections {}
