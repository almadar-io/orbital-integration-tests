//! P-044 Part B: a browser's client kernel running its client dispatches through std-kernel-client (the program's
//! `dispatchKernel`), each step `kernel/step "self"`, answers as its native cascade does.

use orbital_core::client::ClientKernel;
use orbital_core::OrbitalSchema;
use serde_json::{json, Value as Json};
use std::path::{Path, PathBuf};

fn packages(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(rel)
}

fn resolve(schema: OrbitalSchema, path: Option<&Path>) -> OrbitalSchema {
    orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
    let options = orbital_compiler::CompilerOptions::from_env();
    orbital_compiler::phases::lazy::resolve_program_with_path(schema, &options, path).unwrap_or_else(|e| panic!("resolve: {e}")).0
}

fn resolved_lolo(rel: &str) -> Json {
    let path = packages(rel);
    serde_json::to_value(resolve(orbital_test_support::lower_lolo_file(&path).unwrap_or_else(|e| panic!("lower {rel}: {e}")), Some(&path))).unwrap()
}

/// A client app whose Ui counts its bumps, and, with `kernel`, std-kernel-client merged in as `program/open { kernel:
/// "lolo", clientKernel }` merges it: its orbital (app grants made its own, boot page left behind), its types, and the
/// `dispatchKernel` naming it.
fn client_program(kernel: bool) -> OrbitalSchema {
    let mut app = json!({
        "name": "App", "typeContractVersion": 1,
        "orbitals": [{ "name": "Main",
            "entity": { "name": "Cell", "persistence": "runtime", "fields": [{ "name": "id", "type": "string" }, { "name": "n", "type": "number", "default": 0 }] },
            "traits": [{ "name": "Ui", "linkedEntity": "Cell", "dispatch": "client", "stateMachine": {
                "states": [{ "name": "idle", "isInitial": true }, { "name": "bumped" }],
                "events": [{ "key": "BUMP", "name": "Bump" }],
                "transitions": [
                    { "from": "idle", "to": "bumped", "event": "BUMP", "effects": [["set", "@entity.n", ["+", "@entity.n", 1]]] },
                    { "from": "bumped", "to": "bumped", "event": "BUMP", "effects": [["set", "@entity.n", ["+", "@entity.n", 1]]] }] } }],
            "pages": [{ "name": "Home", "path": "/", "traits": [{ "ref": "Ui" }] }] }]
    });
    if kernel {
        let fragment = resolved_lolo("almadar-std/behaviors/lolo/kernel/atoms/std-kernel-client.lolo");
        let grants = fragment["osGrants"].clone();
        for orbital in fragment["orbitals"].as_array().unwrap() {
            let mut orbital = orbital.clone();
            if !grants.is_null() {
                orbital["osGrants"] = grants.clone();
            }
            let pages: Vec<Json> = orbital["pages"].as_array().cloned().unwrap_or_default().into_iter().filter(|p| p.get("protocol").is_none()).collect();
            orbital["pages"] = json!(pages);
            app["orbitals"].as_array_mut().unwrap().push(orbital);
        }
        if let Some(types) = fragment.get("types") {
            app["types"] = types.clone();
        }
        app["dispatchKernel"] = json!({ "trait": "ClientKernel", "mount": ["ClientKernelMachine", "ClientKernel", "ClientKernelSelf"] });
    }
    serde_json::from_value(app).unwrap()
}

fn bumped_twice(kernel: bool) -> (String, Json, ClientKernel) {
    let client = ClientKernel::new(client_program(kernel), None).unwrap_or_else(|e| panic!("load: {e}"));
    for _ in 0..2 {
        client.dispatch("Ui", None, "BUMP", orbital_core::Value::Null).unwrap_or_else(|e| panic!("dispatch: {e}"));
    }
    let ui = client.snapshot_trait("Ui", None);
    (ui.state, serde_json::to_value(&ui.entity).unwrap()["n"].clone(), client)
}

#[test]
fn std_kernel_client_attaches_to_the_browser_kernel_when_the_program_loads() {
    let client = ClientKernel::new(client_program(true), None).unwrap_or_else(|e| panic!("load: {e}"));
    assert_eq!(client.snapshot_trait("ClientKernelSelf", None).state, "attached", "the kernel read its own program");
    assert_eq!(client.snapshot_trait("ClientKernel", None).state, "ready", "the .lolo kernel is ready");
}

#[test]
fn a_ui_event_through_the_lolo_kernel_in_the_browser_ends_where_native_ends() {
    let native = bumped_twice(false);
    assert_eq!((native.0.as_str(), native.1.clone()), ("bumped", json!(2.0)), "control: natively Ui bumped twice");
    let through = bumped_twice(true);
    assert_eq!((through.0.as_str(), through.1.clone()), ("bumped", json!(2.0)), "the .lolo kernel ran both bumps");
    let runs = serde_json::to_value(&through.2.snapshot_trait("ClientKernel", None).entity).unwrap()["runs"].clone();
    assert!(runs.as_f64().is_some_and(|n| n >= 2.0), "both dispatches went through ClientKernel: runs {runs}");
}
