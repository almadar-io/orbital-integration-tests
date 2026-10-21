//! Native snapshot baselines track the shared native-conformance fixtures.

fn conformance_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-schemas/native-conformance")
}

fn sorted_names(dir: &std::path::Path, suffix: &str) -> std::collections::BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|n| n.ends_with(suffix))
        .collect()
}

/// Every iOS baseline PNG maps to a fixture and every fixture has one.
#[test]
fn ios_snapshot_baselines_match_fixtures() {
    let root = conformance_dir();
    let fixtures: std::collections::BTreeSet<String> =
        sorted_names(&root.join("pattern-fixtures"), ".json")
            .into_iter()
            .map(|n| n.trim_end_matches(".json").to_string())
            .filter(|n| !orbital_shell_ios::ios_snapshot_excluded(n))
            .map(|n| format!("test_{}.1.png", n.replace('-', "_")))
            .collect();
    let baselines = sorted_names(&root.join("snapshots/ios"), ".png");
    let stale: Vec<_> = baselines.difference(&fixtures).collect();
    let missing: Vec<_> = fixtures.difference(&baselines).collect();
    assert!(
        stale.is_empty() && missing.is_empty(),
        "stale: {stale:?}\nmissing: {missing:?}"
    );
}

/// Control: the baseline check itself detects both directions.
#[test]
fn baseline_set_difference_flags_stale_and_missing() {
    let fixtures: std::collections::BTreeSet<String> = ["a.png".to_string()].into();
    let baselines: std::collections::BTreeSet<String> = ["b.png".to_string()].into();
    assert_eq!(baselines.difference(&fixtures).count(), 1);
    assert_eq!(fixtures.difference(&baselines).count(), 1);
}

/// Every Android baseline PNG maps to a fixture (or a declared extra snapshot) and each has one.
#[test]
fn android_snapshot_baselines_match_fixtures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-schemas/native-conformance");
    let names = |dir: std::path::PathBuf, suffix: &str| -> std::collections::BTreeSet<String> {
        std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .map(|e| {
                e.expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|n| n.ends_with(suffix))
            .collect()
    };
    let mut fixtures: std::collections::BTreeSet<String> =
        names(root.join("pattern-fixtures"), ".json")
            .into_iter()
            .map(|n| format!("{}.png", n.trim_end_matches(".json")))
            .collect();
    fixtures.extend(
        orbital_shell_android::ANDROID_EXTRA_SNAPSHOTS
            .iter()
            .map(|(name, ..)| format!("{name}.png")),
    );
    let baselines = names(root.join("snapshots/android"), ".png");
    let stale: Vec<_> = baselines.difference(&fixtures).collect();
    let missing: Vec<_> = fixtures.difference(&baselines).collect();
    assert!(
        stale.is_empty() && missing.is_empty(),
        "stale: {stale:?}\nmissing: {missing:?}"
    );
}

/// The registry patterns the Dioxus renderer does not cover, sorted once.
fn dioxus_unsupported(
    registry: &orbital_client_dioxus::registry::DioxusPatternRegistry,
) -> Vec<String> {
    let registry_json: serde_json::Value = serde_json::from_str(
        &orbital_core::language_data::language_data()
            .unwrap_or_else(|e| panic!("{}: {e}", e.code()))
            .patterns,
    )
    .unwrap();
    let patterns = registry_json["patterns"]
        .as_object()
        .expect("patterns object");
    assert!(!patterns.is_empty(), "installed registry has no patterns");
    let mut missing: Vec<String> = patterns
        .keys()
        .filter(|p| !registry.supports(p))
        .cloned()
        .collect();
    missing.sort();
    missing.dedup();
    missing
}

/// A pattern the Dioxus client lacks answers with the declared unsupported renderer.
#[test]
fn dioxus_unsupported_patterns_answer_with_the_declared_error() {
    let registry = orbital_client_dioxus::registry::default_registry();
    for p in &dioxus_unsupported(&registry) {
        assert_eq!(registry.get_renderer(p).pattern_name(), "*", "{p}");
    }
}

#[test]
fn dioxus_coverage_list_is_what_the_registry_answers() {
    let registry = orbital_client_dioxus::registry::default_registry();
    let coverage = registry.coverage();
    assert!(!coverage.is_empty());
    assert!(coverage.windows(2).all(|w| w[0] < w[1]), "sorted, unique");
    assert!(coverage.iter().all(|p| registry.supports(p)));
    assert!(!registry.supports("zz-not-a-pattern"));
    assert_eq!(
        registry.get_renderer("zz-not-a-pattern").pattern_name(),
        "*"
    );
}

/// The native desktop registry (the game overrides register only there), the one
/// `orb-desktop` probes with.
#[test]
fn dioxus_declared_stubs_are_exactly_the_unsupported_patterns() {
    let registry = orbital_client_dioxus::registry::default_registry();
    let unsupported = dioxus_unsupported(&registry);
    let mut stubs: Vec<String> = orbital_client_dioxus::registry::STUBBED_PATTERNS
        .iter()
        .map(|s| s.to_string())
        .collect();
    stubs.sort();
    assert_eq!(
        unsupported, stubs,
        "declare a new gap in STUBBED_PATTERNS, or drop an implemented one"
    );
    assert!(stubs.iter().all(|p| registry.is_stub(p)));
    assert!(
        !registry.is_stub("typography"),
        "control: a supported pattern is no stub"
    );
}
