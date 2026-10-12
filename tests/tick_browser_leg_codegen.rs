//! A compiled client tick whose effects write a browser-stored entity (drift D-1): its data effects
//! leave as one `runTick` leg to the in-process browser store, never dropped. Twin of
//! `orbital-core` `tests/tick_browser_leg.rs` and `@almadar/runtime` `client-role-run-tick`.

use orbital_compiler::phases::resolve_to_oir;
use orbital_core::generate::Backend;
use orbital_core::schema::OrbitalSchema;
use orbital_shell_typescript::TypeScriptBackend;

fn schema(tick_effects: serde_json::Value) -> OrbitalSchema {
    placed(tick_effects, "Draft", false)
}

fn placed(tick_effects: serde_json::Value, linked: &str, background: bool) -> OrbitalSchema {
    serde_json::from_value(serde_json::json!({
        "name": "EditingApp",
        "version": "1.0.0",
        "orbitals": [{
            "name": "Editing",
            "entity": { "name": "Draft", "persistence": "runtime",
                "fields": [{ "name": "id", "type": "string", "required": true }, { "name": "text", "type": "string" }] },
            "auxiliaryEntities": [{ "name": "Note", "persistence": "persistent", "collection": "notes", "local": true,
                "fields": [{ "name": "id", "type": "string", "required": true }, { "name": "text", "type": "string" }] }],
            "traits": [{
                "name": "Editor", "linkedEntity": linked, "category": "interaction",
                "stateMachine": {
                    "states": [{ "name": "editing", "isInitial": true }],
                    "events": [{ "key": "INIT", "name": "INIT" }],
                    "transitions": [{ "from": "editing", "to": "editing", "event": "INIT", "effects": [] }]
                },
                "ticks": [{ "name": "autosave", "interval": 1000, "runsInBackground": background, "effects": tick_effects }]
            }],
            "pages": [{ "name": "Home", "path": "/", "traits": [{ "ref": "Editor" }] }]
        }]
    }))
    .expect("fixture schema must deserialize")
}

fn client_files(schema: &OrbitalSchema) -> Vec<(String, String)> {
    let module = resolve_to_oir(schema).expect("schema resolves to OIR");
    TypeScriptBackend::new()
        .generate(&module)
        .files
        .into_iter()
        .filter(|f| f.path.starts_with("packages/client/"))
        .map(|f| (f.path, f.content))
        .collect()
}

fn containing<'a>(files: &'a [(String, String)], needle: &str) -> Vec<&'a str> {
    files
        .iter()
        .filter(|(_, content)| content.contains(needle))
        .map(|(path, _)| path.as_str())
        .collect()
}

#[test]
fn a_tick_writing_browser_stored_data_posts_a_run_tick_leg_to_the_browser_store() {
    let files = client_files(&schema(serde_json::json!([
        ["persist", "create", "Note", { "text": "@entity.text" }]
    ])));
    assert!(
        !containing(&files, "'Editor\\u0000$TICK_autosave'").is_empty(),
        "the tick is a browser leg of its trait"
    );
    assert!(
        !containing(&files, "runTick: { trait: traitName, tick }").is_empty(),
        "the bridge sends the tick as a runTick request"
    );
    assert!(
        !containing(&files, "sendTick('autosave'").is_empty(),
        "the tick loop hands the firing to the bridge"
    );
}

#[test]
fn control_a_tick_with_only_frame_writes_stays_local() {
    let files = client_files(&schema(serde_json::json!([
        ["set", "@entity.text", "saved"]
    ])));
    assert!(containing(&files, "$TICK_autosave").is_empty(), "no browser leg");
    assert!(containing(&files, "sendTick(").is_empty(), "no tick send");
}

fn server_tick_file(files: &[(String, String)]) -> bool {
    files.iter().any(|(path, content)| {
        path.starts_with("packages/server/") && content.contains("/** Tick `autosave`")
    })
}

fn all_files(schema: &OrbitalSchema) -> Vec<(String, String)> {
    let module = resolve_to_oir(schema).expect("schema resolves to OIR");
    TypeScriptBackend::new()
        .generate(&module)
        .files
        .into_iter()
        .map(|f| (f.path, f.content))
        .collect()
}

/// A trait linked to a browser-stored entity ticks in the browser (its rows live there), and
/// its tick's writes go to the browser store — never to the generated server.
#[test]
fn a_browser_stored_trait_ticks_in_the_client_not_on_the_server() {
    let files = all_files(&placed(
        serde_json::json!([["persist", "create", "Note", { "text": "x" }]]),
        "Note",
        false,
    ));
    assert!(!server_tick_file(&files), "the server must not run a browser-stored trait's tick");
    assert!(!containing(&files, "sendTick('autosave'").is_empty(), "the client tick sends its leg");
}

/// A `[background]` tick runs with no tab open, so it runs on the server even for a
/// `[runtime]`-linked trait.
#[test]
fn a_background_tick_runs_on_the_server() {
    let files = all_files(&placed(
        serde_json::json!([["set", "@entity.text", "saved"]]),
        "Draft",
        true,
    ));
    assert!(server_tick_file(&files), "the server runs a [background] tick");
    assert!(containing(&files, "// Tick: autosave").is_empty(), "no client loop for it");
}

#[test]
fn control_a_runtime_trait_mounted_tick_stays_in_the_client() {
    let files = all_files(&schema(serde_json::json!([["set", "@entity.text", "saved"]])));
    assert!(!server_tick_file(&files));
    assert!(!containing(&files, "// Tick: autosave").is_empty());
}
