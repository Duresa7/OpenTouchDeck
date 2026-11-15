use td_companion::api::Access;

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
