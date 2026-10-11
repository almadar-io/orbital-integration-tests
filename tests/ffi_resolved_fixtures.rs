//! `orbital-ffi` tests boot the kernel from resolved schemas checked in under
//! `crates/orbital-ffi/tests/fixtures/resolved/` (resolving needs the private
//! compiler). Each fixture must equal a fresh resolve of its source program;
//! `ORB_WRITE_FFI_FIXTURES=1` rewrites them.

use std::path::{Path, PathBuf};

const SECOND_ORBITAL: &str = "Chore Management";

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    let path = workspace().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// `01-basic-entity.orb` plus a clone of its orbital under `SECOND_ORBITAL`
/// (entity `Chore`, trait named `second_trait`).
fn two_orbital_program(second_trait: &str) -> String {
    let mut program: serde_json::Value = serde_json::from_str(&read("test-schemas/01-basic-entity.orb")).unwrap();
    let first = program["orbitals"][0].clone();
    let cloned = serde_json::to_string(&first)
        .unwrap()
        .replace("TaskCRUD", "\u{0}TRAIT\u{0}")
        .replace("Task Management", SECOND_ORBITAL)
        .replace("Task", "Chore")
        .replace("tasks", "chores")
        .replace("\u{0}TRAIT\u{0}", second_trait);
    let mut second: serde_json::Value = serde_json::from_str(&cloned).unwrap();
    second["pages"] = serde_json::json!([]);
    program["orbitals"].as_array_mut().unwrap().push(second);
    program.to_string()
}

fn rollback_program() -> String {
    let typography = |content: &str| serde_json::json!({ "type": "typography", "content": content });
    let arm = |event: &str, effects: serde_json::Value| serde_json::json!({ "from": "idle", "to": "idle", "event": event, "effects": effects });
    let machine = |transitions: serde_json::Value| serde_json::json!({ "states": [{ "name": "idle", "isInitial": true }], "transitions": transitions });
    serde_json::json!({
        "name": "rollback",
        "orbitals": [{
            "name": "Main",
            "entity": { "name": "Board", "persistence": "runtime", "shared": true,
                "fields": [{ "name": "id", "type": "string" }, { "name": "score", "type": "number", "default": 0 }] },
            "traits": [
                { "name": "Driver", "linkedEntity": "Board", "stateMachine": machine(serde_json::json!([
                    arm("INIT", serde_json::json!([["render-ui", "main", typography("driver")]])),
                    arm("BUMP", serde_json::json!([["set", "@entity.score", ["+", "@entity.score", 1]]]))
                ])) },
                { "name": "Hud", "linkedEntity": "Board", "stateMachine": machine(serde_json::json!([
                    arm("INIT", serde_json::json!([["render-ui", "sidebar", typography("@entity.score")]]))
                ])) }
            ],
            "pages": [{ "name": "Home", "path": "/", "traits": [{ "ref": "Driver" }, { "ref": "Hud" }] }]
        }]
    })
    .to_string()
}

fn sources() -> Vec<(&'static str, String)> {
    vec![
        ("basic-entity", read("test-schemas/01-basic-entity.orb")),
        ("entity-rerender", read("test-schemas/native-conformance/features/entity-rerender.orb")),
        ("tick-counter", read("test-schemas/native-conformance/features/tick-counter.orb")),
        ("two-orbital-chore", two_orbital_program("ChoreCRUD")),
        ("two-orbital-task", two_orbital_program("TaskCRUD")),
        ("rollback", rollback_program()),
    ]
}

#[test]
fn ffi_resolved_fixtures_match_a_fresh_resolve() {
    let dir = workspace().join("crates/orbital-ffi/tests/fixtures/resolved");
    let write = std::env::var("ORB_WRITE_FFI_FIXTURES").as_deref() == Ok("1");
    let mut stale = Vec::new();
    for (name, program) in sources() {
        let schema = orbital_verify::resolve_schema_json(&program).unwrap_or_else(|e| panic!("resolve {name}: {e}"));
        let fresh = orbital_core::util::canonical_json::to_canonical_string_pretty(&schema).expect("serialize resolved schema");
        let path = dir.join(format!("{name}.json"));
        if write {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &fresh).unwrap();
        } else if std::fs::read_to_string(&path).ok().as_deref() != Some(fresh.as_str()) {
            stale.push(name);
        }
    }
    assert!(stale.is_empty(), "stale ffi fixtures {stale:?}: rerun with ORB_WRITE_FFI_FIXTURES=1");
}

#[test]
fn two_resolves_of_one_program_serialize_the_same_bytes() {
    let program = two_orbital_program("ChoreCRUD");
    let text = || {
        let schema = orbital_verify::resolve_schema_json(&program).expect("resolve");
        orbital_core::util::canonical_json::to_canonical_string_pretty(&schema).expect("serialize")
    };
    assert_eq!(text(), text());
}
