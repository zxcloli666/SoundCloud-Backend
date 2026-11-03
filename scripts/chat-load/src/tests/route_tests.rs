use super::{DEFAULT_MIX, Kind, Routes, assign, check_relay_cap, named_url, parse_mix, per_route};

fn defaults() -> Routes {
    Routes::build(&[], &[], None)
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
            ("r1", Kind::Relay, "https://api.r1.relay.scnative.dev"),
            ("r2", Kind::Relay, "https://api.r2.relay.scnative.dev"),
            ("r4", Kind::Relay, "https://api.r4.relay.scnative.dev"),
            ("p1", Kind::Pro, "https://p1.pro.scnative.dev"),
            ("p2", Kind::Pro, "https://p2.pro.scnative.dev"),
        ]
    );
    assert_eq!(routes.origin, "https://api.scnative.dev");
}

#[test]
fn local_routes_replace_the_defaults() {
    let relays = vec![named_url("r1=http://chat:3000/").expect("a relay")];
    let pros = vec![named_url("p1=http://pro:8080").expect("a pro")];
    let routes = Routes::build(&relays, &pros, Some("http://chat:3000"));
    assert_eq!(routes.list.len(), 2);
    assert_eq!(routes.list[0].base, "http://chat:3000");
    assert!(routes.index_of("direct").is_none());
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
        for (name, weight) in [("r4", 35), ("r2", 30), ("p2", 10), ("r1", 15), ("p1", 10)] {
            let wanted = prefix * weight / 100;
            let got = counts.get(name).copied().unwrap_or(0);
            assert!(got.abs_diff(wanted) <= 1, "{name} {got} of {prefix}");
        }
    }
}

#[test]
fn a_mix_names_known_routes_only() {
    let routes = defaults();
    assert!(parse_mix("direct=1", &routes).is_err());
    assert!(parse_mix("r4=1,r7=1", &routes).is_err());
    assert!(parse_mix("r4=x", &routes).is_err());
    assert!(parse_mix("r4=0", &routes).is_err());
    assert_eq!(parse_mix("r2=3, p1=0", &routes).expect("a mix"), [(1, 3)]);
}

#[test]
fn the_relay_cap_stops_a_plan_that_would_fill_r1() {
    let routes = defaults();
    let mix = parse_mix(DEFAULT_MIX, &routes).expect("a mix");
    assert!(check_relay_cap(&assign(&mix, 20_000), &routes, 9_000).is_ok());
    let heavy = parse_mix("r1=60,r2=40", &routes).expect("a mix");
    assert!(check_relay_cap(&assign(&heavy, 20_000), &routes, 9_000).is_err());
    let pro_only = parse_mix("p1=1,p2=1", &routes).expect("a mix");
    assert!(check_relay_cap(&assign(&pro_only, 20_000), &routes, 9_000).is_ok());
}

#[test]
fn the_night_mix_puts_7000_6000_2000_3000_2000_devices_on_the_five_edges() {
    let routes = defaults();
    let mix = parse_mix(DEFAULT_MIX, &routes).expect("a mix");
    let counts = per_route(&assign(&mix, 20_000), &routes);
    for (name, devices) in [
        ("r4", 7_000),
        ("r2", 6_000),
        ("p2", 2_000),
        ("r1", 3_000),
        ("p1", 2_000),
    ] {
        assert_eq!(counts[name], devices, "{name}");
    }
}
