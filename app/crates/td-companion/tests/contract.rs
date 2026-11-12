use td_companion::{api::Access, bindings};

/// Every endpoint name, frame type and error code appears in the contract as a backticked token.
#[test]
fn api_contract_is_documented() {
    let doc = include_str!("../../../../docs/companion-api.md");
    let documented = |token: &str| doc.contains(&format!("`{token}`"));
    let features = td_companion::api::registry();
    for e in features.iter().flat_map(|f| &f.endpoints) {
        assert!(documented(e.name), "endpoint {}", e.name);
    }
    for e in bindings::events(&features) {
        assert!(documented(e.name), "event {}", e.name);
    }
    for code in bindings::error_codes(&features) {
        assert!(documented(code), "error code {code}");
    }
}
#[test]
fn test_registry_is_consistent() {
    let mut routes = std::collections::HashSet::new();
    let mut open = Vec::new();
    for feature in td_companion::api::registry() {
        for e in feature.endpoints {
            assert!(
                e.name.starts_with(&format!("{}.", feature.name)),
                "{}",
                e.name
            );
            assert!(routes.insert((e.method.to_string(), e.path)), "{}", e.name);
            let params: Vec<_> = e
                .path
                .split('/')
                .filter_map(|s| s.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
                .collect();
            for param in &params {
                assert!(e.path_type.contains(&format!("{param}:")), "{}", e.name);
            }
            assert_eq!(e.path_type.matches(':').count(), params.len(), "{}", e.name);
            if e.access != Access::Session {
                open.push((e.name, e.access));
            }
        }
    }
    open.sort_by_key(|(name, _)| *name);
    // The contract allows exactly these routes without a session.
    assert_eq!(
        open,
        [
            ("auth.pair", Access::Public),
            ("auth.proof", Access::Public),
            ("auth.session_create", Access::Pairing),
            ("system.ping", Access::Public),
        ]
    );
}
