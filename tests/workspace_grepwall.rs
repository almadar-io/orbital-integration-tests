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
        count(&src, "patterns-registry.json\")")
            + count(&src, "services-registry.json\")"),
        "Pillar-F1 gate: a standalone registry.json embed is back — the pattern registry \
         must read the single patterns-registry.json (shared with the validator's embedded view)."
    );
}

/// Pillar F3 — `EFFECT_OPERATORS` is defined once (the sexpr-context base);
/// `effect/utils` DERIVES the effect-position superset from it (`effect_operators()`)
/// instead of re-listing the core effects. And the canonical case converters live
/// uniquely in `orbital_core::str_utils` — the per-crate copies were deleted or now
/// delegate to them (so a core effect / a casing rule can't drift between copies).
#[test]
fn effect_operators_and_casing_single_home() {
    let src = all_workspace_src();
    assert_eq!(
        count(&src, "pub const EFFECT_OPERATORS"),
        1,
        "Pillar-F3 gate: EFFECT_OPERATORS must be defined exactly ONCE (sexpr::operators); \
         the effect/utils superset must derive it via effect_operators(), not re-list it."
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
/// every `crates/*/src/**/*.rs` EXCLUDING `orbital-core/src/str_utils.rs`
/// (the canonical owner) and any test files, with `#[cfg(test)]` blocks
/// stripped from the body of each included file.
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
                if p.contains("orbital-core/src/str_utils.rs") {
                    continue;
                }
                // Exclude files whose name ends with `_test.rs` or `_tests.rs`.
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if stem.ends_with("_test") || stem.ends_with("_tests") {
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
/// `orbital-core/src/str_utils.rs` and any `#[cfg(test)]` / test-file code)
/// and asserts that ZERO `fn <helper>(` definitions exist outside the canonical
/// owner — regardless of whether they are `pub`, `pub(crate)`, or private.
#[test]
fn no_str_utils_helper_forks() {
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
             Canonical owner: crates/orbital-core/src/str_utils.rs."
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
    for entry in fs::read_dir(root.join("crates")).into_iter().flatten().flatten() {
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
        .flat_map(|(path, text)| ts_escape_hatch_uses(text).into_iter().map(move |u| format!("{path}: {u}")))
        .collect();
    assert!(hits.is_empty(), "shell templates use `unknown`/`any` as a type ({} hit(s)):\n{}", hits.len(), hits.join("\n"));
}
