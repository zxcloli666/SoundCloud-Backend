mod budget;
mod providers;
mod vendor;

use serde::Deserialize;

const ANSWER: &str = "{\"say\":\"hello there\"}";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoHostSections {}
