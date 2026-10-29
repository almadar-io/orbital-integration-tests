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

/// The native construct that carries each declared web role, per shell: a pattern's renderer must contain at least one.
/// A role with no entry here fails the test, so a new `@role` cannot ship unmapped.
const IOS_ROLE_EVIDENCE: &[(&str, &[&str])] = &[
    ("button", &["Button("]),
    ("checkbox", &["Toggle(", "accessibilityValue"]),
    ("img", &["accessibilityLabel"]),
    ("navigation", &["accessibilityElement(children: .contain)"]),
    ("progressbar", &["accessibilityValue"]),
    ("radio", &["accessibilityAddTraits"]),
    ("radiogroup", &["accessibilityElement(children: .contain)"]),
    ("separator", &["accessibilityHidden", "Divider("]),
    ("status", &["accessibilityLabel", "accessibilityElement"]),
    ("switch", &["Toggle(", "accessibilityValue"]),
    ("tablist", &["accessibilityAddTraits"]),
    ("textbox", &["TextField(", "TextEditor(", "AtomTextEntry("]),
];

const ANDROID_ROLE_EVIDENCE: &[(&str, &[&str])] = &[
    ("button", &["Button("]),
    ("checkbox", &["Checkbox(", "Role.Checkbox"]),
    ("img", &["almadarLabel", "contentDescription"]),
    ("navigation", &["almadarLandmark"]),
    ("progressbar", &["progressBarRangeInfo"]),
    ("radio", &["RadioButton(", "Role.RadioButton"]),
    ("radiogroup", &["selectableGroup"]),
    ("separator", &["HorizontalDivider", "VerticalDivider"]),
    ("status", &["almadarStatus", "liveRegion"]),
    ("switch", &["Switch(", "Role.Switch"]),
    ("tablist", &["Role.Tab"]),
    ("textbox", &["TextField("]),
];

fn carries_role(evidence: &[(&str, &[&str])], role: &str, source: &str) -> bool {
    let (_, markers) = evidence
        .iter()
        .find(|(r, _)| *r == role)
        .unwrap_or_else(|| panic!("role `{role}` is declared by a web pattern but has no native evidence entry"));
    markers.iter().any(|m| source.contains(m))
}

fn roles_missing(pack: &orbital_core::renderer_packs::RendererPack, ext: &str, evidence: &[(&str, &[&str])]) -> Vec<String> {
    let shell = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../orbital-shell-typescript");
    let mut missing = Vec::new();
    for (pattern, role) in orbital_test_support::declared_pattern_roles(&shell) {
        let file = format!("{}.{ext}", pattern.replace('-', "_"));
        let source = pack
            .sources
            .iter()
            .find(|(rel, _)| rel.rsplit('/').next() == Some(file.as_str()))
            .map(|(_, src)| src.as_str());
        match source {
            None => missing.push(format!("{pattern}: no renderer file {file} in the pack")),
            Some(src) if !carries_role(evidence, &role, src) => missing.push(format!("{pattern} ({role})")),
            Some(_) => {}
        }
    }
    missing
}

#[test]
fn the_role_check_sees_a_renderer_without_the_role() {
    assert!(!carries_role(IOS_ROLE_EVIDENCE, "status", "Text(\"x\")"));
    assert!(carries_role(IOS_ROLE_EVIDENCE, "status", "Text(\"x\").accessibilityLabel(\"x\")"));
    assert!(!carries_role(ANDROID_ROLE_EVIDENCE, "navigation", "Column(modifier = modifier.almadarLabel(\"x\"))"));
    assert!(carries_role(ANDROID_ROLE_EVIDENCE, "navigation", "Column(modifier = modifier.almadarLandmark(\"x\"))"));
}

#[test]
fn declared_roles_include_the_patterns_the_web_conformance_covers() {
    let shell = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../orbital-shell-typescript");
    let roles = orbital_test_support::declared_pattern_roles(&shell);
    assert_eq!(roles.get("switch").map(String::as_str), Some("switch"));
    assert_eq!(roles.get("alert").map(String::as_str), Some("status"));
    assert!(roles.len() >= 21, "the web conformance declares 21+ patterns, found {}", roles.len());
}

/// Every pattern the web declares a role for renders that role natively on iOS.
#[test]
fn ios_renderers_carry_the_declared_roles() {
    let pack = orbital_shell_ios::require_renderer_pack().expect("ios renderer pack");
    assert_eq!(roles_missing(pack, "swift", IOS_ROLE_EVIDENCE), Vec::<String>::new());
}

/// Every pattern the web declares a role for renders that role natively on Android.
#[test]
fn android_renderers_carry_the_declared_roles() {
    let pack = orbital_shell_android::require_renderer_pack().expect("android renderer pack");
    assert_eq!(roles_missing(pack, "kt", ANDROID_ROLE_EVIDENCE), Vec::<String>::new());
}
