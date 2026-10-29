//! Workspace-wide single-owner gates: a canonical helper or embedded registry
//! must have exactly one home across every crate, and no shell template may
//! use `unknown`/`any` as a type.

use std::fs;
use std::path::Path;

#[path = "../../orbital-shell-typescript/tests/common/grep_helpers.rs"]
mod grep_helpers;
use grep_helpers::{count, strip_test_blocks, ts_escape_hatch_uses};

/// Walk every `crates/*/src/**.rs` in the WHOLE workspace (not just this crate),
/// so cross-crate single-embed invariants can be asserted.
fn all_workspace_src() -> String {
    fn walk(dir: &Path, out: &mut String) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
                && path.to_string_lossy().contains("/src/")
            {
                out.push_str(&fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    // CARGO_MANIFEST_DIR = <ws>/crates/orbital-shell-typescript → parent = <ws>/crates
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .to_path_buf();
    let mut out = String::new();
    walk(&crates, &mut out);
    out
}

/// Pillar F1 — the pattern vocabulary is embedded NOWHERE. `component-mapping.json`,
/// `event-contracts.json`, `patterns-registry.json` and `services-registry.json` are
/// read at runtime from the installed `@almadar/core` (`orbital_core::language_data`),
/// so no crate may `include_str!`/`include_bytes!` them. This is the workspace-wide
/// successor to the per-shell triplication that let codegen and the validator drift.
#[test]
fn registry_json_embedded_once() {
    let src = all_workspace_src();
    for file in [
        "component-mapping.json",
        "event-contracts.json",
        "patterns-registry.json",
        "services-registry.json",
    ] {
        assert_eq!(
            count(&src, &format!("include_str!(\"baked/{file}\")")),
            0,
            "Pillar-F1 gate: {file} must not be include_str!'d — it is installed language data \
             (orbital_core::language_data)."
        );
        assert_eq!(
            count(&src, &format!("baked-gz/{file}.gz")),
            0,
            "Pillar-F1 gate: {file} must not be embedded gzipped — it is installed language data."
        );
    }
    // Every `registry.json")` occurrence must be the tail of `patterns-registry.json")` or
    // `services-registry.json")` — a DIFFERENT artifact from the pattern registry:
    // orbital-cli's runtime path-joins to the services registry, not a pattern-registry embed. A standalone
    // `registry.json` embed (the old per-shell copies) makes the counts diverge.
    assert_eq!(
        count(&src, "registry.json\")"),
        count(&src, "patterns-registry.json\")") + count(&src, "services-registry.json\")"),
        "Pillar-F1 gate: a standalone registry.json embed is back — the pattern registry \
         must read the single patterns-registry.json (shared with the validator's embedded view)."
    );
}

/// Pillar F3 — the effect operators are read from the operator table's `effect` category (no hand list);
/// `effect/utils` DERIVES the effect-position superset from it (`effect_operators()`). And the canonical case converters live
/// uniquely in `orbital_core::str_utils` — the per-crate copies were deleted or now
/// delegate to them (so a core effect / a casing rule can't drift between copies).
#[test]
fn effect_operators_and_casing_single_home() {
    let src = all_workspace_src();
    assert_eq!(
        count(&src, "pub const EFFECT_OPERATORS"),
        0,
        "Pillar-F3 gate: the effect operators are the table's `effect` category (sexpr::operators::effect_category_operators); \
         a hand-written EFFECT_OPERATORS list is back."
    );
    for canon in [
        "pub fn to_snake_case(",
        "pub fn to_kebab_case(",
        "pub fn to_kebab_case_simple(",
        "pub fn capitalize_first(",
        "pub fn to_pascal_case(",
        "pub fn to_title_case(",
        "pub fn escape_js_string(",
    ] {
        assert_eq!(
            count(&src, canon),
            1,
            "Pillar-F3 gate: `{canon}` must be defined exactly once (orbital_core::str_utils). \
             A duplicate canonical case converter is back."
        );
    }
}

/// Pillar F4 — the render-ui effect extractors live once in
/// `render_ui::item_actions`. The byte-identical re-spellings in `slot.rs` and
/// `trait_validator.rs` were deleted and now import the canonical helpers, so a
/// fix to "what is a render-ui effect / its slot" can't land in one rule and
/// miss its siblings.
#[test]
fn render_ui_extractors_single_home() {
    let src = all_workspace_src();
    assert_eq!(
        count(&src, "fn is_render_ui_effect("),
        1,
        "Pillar-F4 gate: is_render_ui_effect must be defined once (render_ui::item_actions)."
    );
    assert_eq!(
        count(&src, "fn extract_slot("),
        1,
        "Pillar-F4 gate: extract_slot must be defined once (render_ui::item_actions)."
    );
}

/// Build the source corpus used by the str_utils fork gate:
/// every `crates/*/src/**/*.rs` EXCLUDING `orbital-core/crates/base/src/str_utils.rs`
/// (the canonical owner) and any test files, with `#[cfg(test)]` blocks
/// stripped from the body of each included file.
/// The canonical string-helper owner, relative to the workspace's `crates/`.
const STR_UTILS_OWNER: &str = "orbital-core/crates/base/src/util/str_utils.rs";

fn workspace_src_no_str_utils_no_tests() -> String {
    fn walk(dir: &Path, out: &mut String) {
        let mut entries: Vec<_> = fs::read_dir(dir).into_iter().flatten().flatten().collect();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                // Skip `tests/` directories entirely — those are integration test
                // files, not production source.
                if path.file_name().and_then(|n| n.to_str()) == Some("tests") {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
                && path.to_string_lossy().contains("/src/")
            {
                let p = path.to_string_lossy();
                // Exclude the canonical owner — it is allowed to define these fns.
                if p.contains(STR_UTILS_OWNER) {
                    continue;
                }
                // Exclude files whose name ends with `_test.rs` or `_tests.rs`.
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if stem == "tests" || stem.ends_with("_test") || stem.ends_with("_tests") {
                        continue;
                    }
                }
                let body = fs::read_to_string(&path).unwrap_or_default();
                out.push_str(&strip_test_blocks(&body));
            }
        }
    }
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .to_path_buf();
    let mut out = String::new();
    walk(&crates, &mut out);
    out
}

/// Pillar G — canonical string-helper fork gate.
///
/// The helpers in `orbital_core::str_utils` are the ONE source of truth for
/// case-conversion and string-escaping.  Private re-implementations are as
/// harmful as public ones: they silently diverge on edge cases and make
/// cross-cutting fixes (e.g. acronym handling) miss callers.
///
/// This test scans every production `.rs` file in the workspace (excluding
/// the canonical owner `STR_UTILS_OWNER` and any `#[cfg(test)]` / test-file code)
/// and asserts that ZERO `fn <helper>(` definitions exist outside the canonical
/// owner — regardless of whether they are `pub`, `pub(crate)`, or private.
#[test]
fn no_str_utils_helper_forks() {
    let owner = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(STR_UTILS_OWNER);
    assert!(
        owner.is_file(),
        "the canonical owner moved: {} does not exist",
        owner.display()
    );
    let src = workspace_src_no_str_utils_no_tests();
    for banned in [
        "fn to_snake_case(",
        "fn to_kebab_case(",
        "fn to_camel_case(",
        "fn to_pascal_case(",
        "fn to_title_case(",
        "fn capitalize_first(",
        "fn kebab_to_pascal(",
        "fn split_to_pascal_case(",
        "fn split_to_camel_case(",
        "fn escape_js_string(",
        "fn escape_python_string(",
        "fn escape_double_quoted(",
        "fn sanitize_identifier(",
    ] {
        assert_eq!(
            count(&src, banned),
            0,
            "Pillar-G gate: `{banned}` is defined outside `orbital_core::str_utils` — \
             route to the canonical implementation there instead of adding a local copy \
             (even a private one that merely delegates). \
             Canonical owner: crates/orbital-core/crates/base/src/util/str_utils.rs."
        );
    }
}

/// Every `.ts`/`.tsx` under the workspace's static shell templates
/// (each shell crate's `template/` / `templates/`), which are copied verbatim into generated apps.
fn template_sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().and_then(|n| n.to_str()) != Some("node_modules") {
                    walk(&path, out);
                }
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("ts" | "tsx")
            ) {
                out.push((
                    path.display().to_string(),
                    fs::read_to_string(&path).unwrap_or_default(),
                ));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    for entry in fs::read_dir(root.join("crates"))
        .into_iter()
        .flatten()
        .flatten()
    {
        for dir in ["templates"] {
            let template = entry.path().join(dir);
            if template.is_dir() {
                walk(&template, &mut out);
            }
        }
    }
    out
}

/// The static shell templates are copied verbatim into generated apps, so they
/// carry the same no-`unknown`/`any` rule as the emitter (`gate_grepwall.rs`).
#[test]
fn shell_templates_have_no_unknown_or_any() {
    let templates = template_sources();
    assert!(!templates.is_empty(), "no shell templates found");
    let hits: Vec<String> = templates
        .iter()
        .flat_map(|(path, text)| {
            ts_escape_hatch_uses(text)
                .into_iter()
                .map(move |u| format!("{path}: {u}"))
        })
        .collect();
    assert!(
        hits.is_empty(),
        "shell templates use `unknown`/`any` as a type ({} hit(s)):\n{}",
        hits.len(),
        hits.join("\n")
    );
}

/// EXEC rules no unit test can assert: ownership and design rules, or ruled but not built.
const EXEC_DESIGN_RULES: &[(&str, &str)] = &[
    ("EXEC-BOUND-1", "ownership order is a design rule"),
    (
        "EXEC-BOUND-2",
        "a host deciding nothing is checked per mechanism, not by one test",
    ),
    ("EXEC-BOUND-3", "nothing implicit is a design rule"),
    (
        "EXEC-BOUND-4",
        "ownership of a new mechanism is a design rule",
    ),
    (
        "EXEC-BOUND-5",
        "one implementation is enforced by the single-owner gates above",
    ),
    ("EXEC-EVAL-3", "design rule"),
    ("EXEC-CONC-7", "the guest kernel part is ruled, not built"),
    ("EXEC-PERF-1", "ruled, not built (P-003)"),
    ("EXEC-PERF-2", "ruled, not built (P-003)"),
];

/// EXEC rules proven outside orbital-core: the test that proves each.
const EXEC_RULES_PROVEN_ELSEWHERE: &[(&str, &str)] = &[
    (
        "EXEC-ENDPOINT-2",
        "orbital-compiler phases/validation/ui/page.rs",
    ),
    (
        "EXEC-ENDPOINT-4",
        "orbital-compiler phases/validation/ui/page.rs",
    ),
    (
        "EXEC-STORE-1",
        "std spec/store/firestore-twin.replay.goldens.json",
    ),
    (
        "EXEC-STORE-2",
        "std spec/store/postgres-twin.replay.goldens.json",
    ),
    (
        "EXEC-STORE-3",
        "std spec/store/couchdb-twin.replay.goldens.json",
    ),
    (
        "EXEC-LANG-3",
        "std spec/cli/lint-user-identity.replay.goldens.json",
    ),
    (
        "EXEC-SEED-5",
        "host twin: needs a host test outside orbital-core",
    ),
    (
        "EXEC-CONC-8",
        "host twin: the JS host's lane differs (defect list)",
    ),
    (
        "EXEC-QUEUE-4",
        "std spec/codec/semver-request-id.replay.goldens.json",
    ),
    (
        "EXEC-SEED-6",
        "orbital-shell-typescript hosts/node/test/runtime-parity/kernel-engine.test.ts",
    ),
];

/// EXEC rules orbital-core does not cite yet (P-014 Phase 3). Shrinks only: citing one makes this gate
/// fail until its entry is deleted.
const EXEC_RULES_PENDING: &[&str] = &[
    // Not observable in the kernel; each needs a test where it lives.
    "EXEC-ENDPOINT-6", // page `access`: shell-web PageAccessHost and the std verification atoms
    "EXEC-RENDER-5",   // toast slot UI: the web renderer
    "EXEC-EVAL-5",     // static argument typing: `orb validate`
    "EXEC-LANG-1",     // brackets are data: the orbital-lolo parser
    "EXEC-CONC-3",     // keyed completions on real host threads: the native os host
    "EXEC-CONC-6",     // os/spawn, os/wait, os/kill and std-supervisor: the native os host and std
    "EXEC-CONC-0a",    // the explicit-control model; primitives 2 and 3 are not implemented
    "EXEC-HTTP-1",     // the http driver is archived; the std http atom's twin parity proves it
    "EXEC-HTTP-4",     // the redis driver is archived; the std redis atom's twin parity proves it
    "EXEC-HTTP-5",     // the postgres driver is archived; the std postgres atom's twin parity proves it
];

/// Every `**EXEC-<FAMILY>-<N> - …**` rule heading in the execution-semantics spec.
fn exec_rule_ids() -> Vec<String> {
    let spec =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/Almadar_Execution_Semantics.md");
    let text = fs::read_to_string(&spec).unwrap_or_else(|e| panic!("read {}: {e}", spec.display()));
    text.lines()
        .filter_map(|line| line.strip_prefix("**EXEC-"))
        .filter_map(|rest| rest.split_once(" - ").map(|(id, _)| format!("EXEC-{id}")))
        .collect()
}

/// orbital-core's test code: files under `tests/`, `tests.rs`/`*_tests.rs`, and what follows a file's first `#[cfg(test)]`.
fn core_test_code() -> String {
    fn walk(dir: &Path, out: &mut String) {
        for path in fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
        {
            if path.is_dir() {
                walk(&path, out);
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !name.ends_with(".rs") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap_or_default();
            let in_tests = path.components().any(|c| c.as_os_str() == "tests")
                || name == "tests.rs"
                || name.ends_with("_tests.rs");
            out.push_str(if in_tests {
                &text[..]
            } else {
                text.find("#[cfg(test)]").map_or("", |i| &text[i..])
            });
            out.push('\n');
        }
    }
    let mut out = String::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../orbital-core/crates"),
        &mut out,
    );
    out
}

/// A rule is cited by its id (not followed by a letter or digit, so `EXEC-ENDPOINT-3` is not `EXEC-ENDPOINT-3a`)
/// or by a test named after it (`fn exec_endpoint_3a_…`).
fn cites(test_code: &str, id: &str) -> bool {
    let by_id = test_code
        .match_indices(id)
        .any(|(i, _)| !test_code[i + id.len()..].starts_with(|c: char| c.is_ascii_alphanumeric()));
    let fn_prefix = format!("fn {}_", id.to_ascii_lowercase().replace('-', "_"));
    by_id || test_code.contains(&fn_prefix)
}

#[test]
fn every_exec_rule_is_cited_by_a_core_test_or_listed_with_its_reason() {
    let rules = exec_rule_ids();
    assert!(
        rules.len() >= 100,
        "the spec's rule headings were not found ({} read)",
        rules.len()
    );
    let test_code = core_test_code();
    let cited: std::collections::BTreeSet<&str> = rules
        .iter()
        .map(String::as_str)
        .filter(|id| cites(&test_code, id))
        .collect();
    let listed = |id: &str| {
        EXEC_DESIGN_RULES
            .iter()
            .chain(EXEC_RULES_PROVEN_ELSEWHERE)
            .any(|(r, _)| *r == id)
            || EXEC_RULES_PENDING.contains(&id)
    };
    let uncovered: Vec<&String> = rules
        .iter()
        .filter(|r| !cited.contains(r.as_str()) && !listed(r))
        .collect();
    assert!(
        uncovered.is_empty(),
        "EXEC rules with no citing orbital-core test and no listed reason: {uncovered:?}"
    );
    let stale: Vec<&&str> = EXEC_RULES_PENDING
        .iter()
        .filter(|r| cited.contains(**r))
        .collect();
    assert!(
        stale.is_empty(),
        "now cited, delete from EXEC_RULES_PENDING: {stale:?}"
    );
    let unknown: Vec<&str> = EXEC_DESIGN_RULES
        .iter()
        .chain(EXEC_RULES_PROVEN_ELSEWHERE)
        .map(|(r, _)| *r)
        .chain(EXEC_RULES_PENDING.iter().copied())
        .filter(|r| !rules.iter().any(|x| x == r))
        .collect();
    assert!(
        unknown.is_empty(),
        "listed rules the spec no longer has: {unknown:?}"
    );
    let mut seen = std::collections::BTreeSet::new();
    let twice: Vec<&str> = EXEC_DESIGN_RULES
        .iter()
        .chain(EXEC_RULES_PROVEN_ELSEWHERE)
        .map(|(r, _)| *r)
        .chain(EXEC_RULES_PENDING.iter().copied())
        .filter(|r| !seen.insert(*r))
        .collect();
    assert!(
        twice.is_empty(),
        "a rule is listed more than once: {twice:?}"
    );
}
