use super::{DEFAULT_MIX, Kind, Routes, assign, check_relay_cap, named_url, parse_mix, per_route};

fn defaults() -> Routes {
    Routes::build(None, &[], &[], None)
}

#[test]
fn default_routes_follow_the_desktop_edge_names() {
    let routes = defaults();
    let shown: Vec<(&str, Kind, &str)> = routes
        .list
        .iter()
        .map(|route| (route.name.as_str(), route.kind, route.base.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            ("direct", Kind::Direct, "https://api.scnative.dev"),
            ("r1", Kind::Relay, "https://api.r1.relay.scnative.dev"),
            ("r2", Kind::Relay, "https://api.r2.relay.scnative.dev"),
            ("p1", Kind::Pro, "https://p1.pro.scnative.dev"),
        ]
    );
    assert_eq!(routes.origin, "https://api.scnative.dev");
}

#[test]
fn local_routes_replace_the_defaults() {
    let relays = vec![named_url("r1=http://chat:3000/").expect("a relay")];
    let pros = vec![named_url("p1=http://pro:8080").expect("a pro")];
    let routes = Routes::build(
        Some("http://chat:3000"),
        &relays,
        &pros,
        Some("http://chat:3000"),
    );
    assert_eq!(routes.list.len(), 3);
    assert_eq!(routes.list[1].base, "http://chat:3000");
    assert!(named_url("direct=http://x").is_err());
    assert!(named_url("r9").is_err());
    assert!(named_url("r9=ftp://x").is_err());
}

#[test]
fn every_prefix_of_the_assignment_keeps_the_mix() {
    let routes = defaults();
    let mix = parse_mix(DEFAULT_MIX, &routes).expect("a mix");
    let assignment = assign(&mix, 20_000);
    for prefix in [20usize, 100, 2_000, 5_000, 20_000] {
        let counts = per_route(&assignment[..prefix], &routes);
        for (name, weight) in [("direct", 25), ("r1", 25), ("r2", 40), ("p1", 10)] {
            let wanted = prefix * weight / 100;
            let got = counts.get(name).copied().unwrap_or(0);
            assert!(got.abs_diff(wanted) <= 1, "{name} {got} of {prefix}");
        }
    }
}

#[test]
fn a_mix_names_known_routes_only() {
    let routes = defaults();
    assert!(parse_mix("direct=1,r7=1", &routes).is_err());
    assert!(parse_mix("direct=x", &routes).is_err());
    assert!(parse_mix("direct=0", &routes).is_err());
    assert_eq!(parse_mix("r2=3, p1=0", &routes).expect("a mix"), [(2, 3)]);
}

#[test]
fn the_relay_cap_stops_a_plan_that_would_fill_r1() {
    let routes = defaults();
    let mix = parse_mix(DEFAULT_MIX, &routes).expect("a mix");
    assert!(check_relay_cap(&assign(&mix, 20_000), &routes, 9_000).is_ok());
    let heavy = parse_mix("r1=60,r2=40", &routes).expect("a mix");
    assert!(check_relay_cap(&assign(&heavy, 20_000), &routes, 9_000).is_err());
    let direct = parse_mix("direct=1", &routes).expect("a mix");
    assert!(check_relay_cap(&assign(&direct, 20_000), &routes, 9_000).is_ok());
}
