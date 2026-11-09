//! The launchpad catalog opens a behavior as a guest (`program/open`); `orb serve` registers it on a host and mounts its
//! pages. This sweep runs both in one process for each behavior of `CATALOG_PARITY_LIST` and prints one JSON line per
//! divergence of what the page paints, the rows its traits hold, and what a mount or a painted control's event emits and
//! moves. Nothing is written to disk and no server, port or browser is started.
//!
//! `CATALOG_PARITY_LIST`  newline list of absolute `.lolo` / resolved `.orb` paths (unset: the test is skipped)
//! `CATALOG_PARITY_SHARD` `i/n`: this process takes the behaviors whose index modulo `n` is `i`
//! `CATALOG_PARITY_GRANTS` a file caching the launchpad guest orbital's grants (written on first use, read after)

use orbital_core::language::pattern_props::PATTERN_EVENT_NAME_PROPS;
use orbital_core::OrbitalSchema;
use orbital_shell_rust::host::boot::{boot, BootEnv};
use orbital_shell_rust::host::run::{ProtocolSide, RUN_ENTITY};
use orbital_shell_rust::host::store::DriverAdapter;
use orbital_shell_rust::host::{Host, HostConfig, ProcessEventResult};
use serde_json::{json, Value as Json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const BEHAVIOR_BUDGET: Duration = Duration::from_secs(600);
const COMPOSE_NOW: &str = "1970-01-01T00:00:00.000Z";
const SEED_MARKS: &str = "mock_seed_marks";
const SEED_BEHAVIOR: &str = "std/behaviors/std-mock-page-seed";
const GUEST_PLACEHOLDER: &str = "<guest>";
const FIRING_KINDS: [&str; 3] = ["event", "event-list", "event-map"];
const LAUNCHPAD: &str = "packages/almadar-std/behaviors/lolo/dev/organisms/std-launchpad.lolo";
const GUEST_SESSION_TRAIT: &str = "GuestSession";
const SEEDER_TRAIT: &str = "MockDataMockSeed";

/// The directory `orb` runs from in the monorepo (its working directory), where package discovery starts.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn packages() -> Result<Vec<orbital_core::behavior_packages::BehaviorPackage>, String> {
    static LOWERER: std::sync::Once = std::sync::Once::new();
    LOWERER.call_once(|| orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path)));
    let start = workspace_root();
    let user_store = orbital_core::behavior_packages::user_store_dir().unwrap_or_else(|| start.clone());
    orbital_core::behavior_packages::discover(&start, &user_store).map_err(|e| format!("behavior package discovery: {e:?}"))
}

fn compiler_options() -> Result<orbital_compiler::CompilerOptions, String> {
    let mut compiler = orbital_compiler::CompilerOptions::from_env();
    compiler.inline.loader_config.packages = packages()?;
    Ok(compiler)
}

/// What `program/compose` loads a `uses` from: the behavior the loader resolves, as orb's `behavior/source` reads it.
fn source_of(spec: &str) -> Result<orbital_core::Value, String> {
    use orbital_compiler::loader::{ExternalOrbitalLoader, ImportChain, LoaderConfig};
    let mut loader = ExternalOrbitalLoader::new(LoaderConfig { base_path: workspace_root(), packages: packages()?, ..Default::default() });
    let loaded = loader.load(spec, None, &mut ImportChain::new()).map_err(|e| format!("behavior/source: cannot resolve `{spec}`: {e}"))?;
    let value = serde_json::to_value(&loaded.schema).map_err(|e| e.to_string())?;
    value_of(value)
}

fn resolve(path: &Path) -> Result<OrbitalSchema, String> {
    let compiler = compiler_options()?;
    let schema = orbital_compiler::phases::parse_file(path).map_err(|e| format!("parse {}: {e:?}", path.display()))?;
    orbital_compiler::phases::resolve_compiled_program_with_path(schema, &compiler, Some(path)).map_err(|e| format!("resolve {}: {e:?}", path.display()))
}

/// `std/behaviors/std-mock-page-seed` resolved, as the catalog's `behavior/resolve` and serve's `program/read` resolve it.
fn seed_program() -> Result<Json, String> {
    use orbital_compiler::loader::{ExternalOrbitalLoader, ImportChain, LoaderConfig};
    let mut loader = ExternalOrbitalLoader::new(LoaderConfig { base_path: workspace_root(), packages: packages()?, ..Default::default() });
    let loaded = loader.load(SEED_BEHAVIOR, None, &mut ImportChain::new()).map_err(|e| format!("cannot resolve `{SEED_BEHAVIOR}`: {e}"))?;
    serde_json::to_value(resolve(&loaded.source_path)?).map_err(|e| e.to_string())
}

/// What the catalog's guest session may hand a guest: its orbital's grants with the launchpad's app level (EXEC-GRANTS-2).
fn launchpad_guest_grants() -> Result<Json, String> {
    let launchpad = resolve(&workspace_root().join(LAUNCHPAD))?;
    let orbital = launchpad
        .orbitals
        .iter()
        .find(|o| o.traits.iter().any(|t| t.trait_name() == Some(GUEST_SESSION_TRAIT)))
        .ok_or_else(|| format!("{LAUNCHPAD} composes no {GUEST_SESSION_TRAIT} trait"))?;
    let none = orbital_core::runtime::os_grants::OsGrants::default();
    let held = orbital_core::runtime::os_grants::union(launchpad.os_grants.as_ref().unwrap_or(&none), orbital.os_grants.as_ref().unwrap_or(&none));
    serde_json::to_value(held).map_err(|e| e.to_string())
}

fn holds_seed_marks(program: &Json) -> bool {
    let items = |value: &Json, key: &str| value.get(key).and_then(Json::as_array).cloned().unwrap_or_default();
    items(program, "orbitals")
        .iter()
        .any(|orbital| items(orbital, "auxiliaryEntities").iter().any(|entity| entity.get("collection").and_then(Json::as_str) == Some(SEED_MARKS)))
}

fn value_of(json: Json) -> Result<orbital_core::Value, String> {
    serde_json::from_value(json).map_err(|e| e.to_string())
}

fn compose_with_seed(program: &OrbitalSchema, seed: &Json) -> Result<OrbitalSchema, String> {
    let programs = vec![value_of(serde_json::to_value(program).map_err(|e| e.to_string())?)?, value_of(seed.clone())?];
    let composed = orbital_compiler::composition::compose_programs(programs, &program.name, None, None, None, COMPOSE_NOW, &source_of)?;
    serde_json::from_value(orbital_core::json_value::value_to_json(&composed)).map_err(|e| e.to_string())
}

#[derive(Clone, PartialEq)]
struct Capture {
    paints: Json,
    emitted: Json,
    outcomes: Json,
}

fn served_capture(outcomes: &[(String, ProcessEventResult)], seed_trait: &str, event: &str) -> Capture {
    let response = orbital_core::EventResponse::from_process_results(outcomes, seed_trait, event);
    let paints: Vec<Json> = response
        .client_effects_by_trait
        .iter()
        .flatten()
        .filter_map(|paint| match &paint.effect {
            orbital_core::ClientEffect::RenderUi { slot, config } => Some(json!({
                "slot": slot,
                "trait": paint.trait_name,
                "content": config.as_ref().map_or(Json::Null, orbital_core::json_value::value_to_json),
            })),
            _ => None,
        })
        .collect();
    let emitted: Vec<Json> = response
        .emitted_events
        .iter()
        .map(|e| json!({ "event": e.event, "payload": e.payload.as_ref().map_or(Json::Null, orbital_core::json_value::value_to_json) }))
        .collect();
    let moved: Vec<Json> = outcomes
        .iter()
        .map(|(trait_name, result)| json!({ "trait": trait_name, "event": result.event, "from": result.previous_state, "to": result.new_state, "fired": result.executed }))
        .collect();
    Capture { paints: Json::Array(paints), emitted: Json::Array(emitted), outcomes: Json::Array(moved) }
}

/// The one place a guest answer is made comparable to a served response: the guest answer's `orbital` and `embedded`
/// attributions are dropped because a served response carries none.
fn guest_capture(answer: &Json) -> Capture {
    let list = |key: &str| answer.get(key).and_then(Json::as_array).cloned().unwrap_or_default();
    let paints: Vec<Json> = list("rendered")
        .iter()
        .map(|p| json!({ "slot": p["slot"], "trait": p["trait"], "content": p["content"] }))
        .collect();
    Capture { paints: Json::Array(paints), emitted: Json::Array(list("emitted")), outcomes: Json::Array(list("outcomes")) }
}

fn mask_guest(value: &mut Json, handle: &str) {
    match value {
        Json::String(text) if !handle.is_empty() && text == handle => *text = GUEST_PLACEHOLDER.to_string(),
        Json::Array(items) => items.iter_mut().for_each(|item| mask_guest(item, handle)),
        Json::Object(map) => map.values_mut().for_each(|item| mask_guest(item, handle)),
        _ => {}
    }
}

/// A guest refusal carries the op that refused as a prefix; a served one does not.
fn guest_error(failed: &Json) -> String {
    let message = failed.get("error").and_then(Json::as_str).map_or_else(|| failed.to_string(), str::to_string);
    for op in ["program/open: ", "program/dispatch: ", "program/inputs: "] {
        if let Some(rest) = message.strip_prefix(op) {
            return rest.to_string();
        }
    }
    message
}

struct Opener {
    host: Arc<Host>,
}

impl Opener {
    fn new(grants: &Json) -> Result<Self, String> {
        let emit = json!({ "emit": { "success": "ANSWERED", "failure": "FAILED" } });
        let schema: OrbitalSchema = serde_json::from_value(json!({
            "name": "CatalogParityOpener",
            "typeContractVersion": 1,
            "osGrants": grants,
            "orbitals": [{
                "name": "Opener",
                "entity": { "name": "Box", "persistence": "runtime", "fields": [
                    { "name": "id", "type": "string" }, { "name": "guest", "type": "string" },
                    { "name": "answer", "type": "object", "nullable": true }, { "name": "failed", "type": "object", "nullable": true },
                    { "name": "ticks", "type": "array", "default": [] }] },
                "traits": [{
                    "name": "Opener", "linkedEntity": "Box",
                    "stateMachine": {
                        "states": [{ "name": "idle", "isInitial": true }],
                        "transitions": [
                            { "from": "idle", "to": "idle", "event": "RESET", "effects": [
                                ["set", "@entity.answer", null], ["set", "@entity.failed", null]] },
                            { "from": "idle", "to": "idle", "event": "OPEN", "effects": [
                                ["program/open", "@payload.program", { "route": "@payload.route", "declared": true }, { "emit": { "success": "OPENED", "failure": "FAILED" } }]] },
                            { "from": "idle", "to": "idle", "event": "OPENED", "effects": [
                                ["set", "@entity.guest", "@payload.result.guest"],
                                ["set", "@entity.answer", "@payload.result"],
                                ["set", "@entity.failed", null]] },
                            { "from": "idle", "to": "idle", "event": "CALL", "effects": [
                                ["program/dispatch", "@payload.address", "@payload.payload", { "guest": "@entity.guest", "emit": emit["emit"] }]] },
                            { "from": "idle", "to": "idle", "event": "LIVE", "effects": [
                                ["program/live", { "kind": "guest", "guest": "@entity.guest" }, { "key": "probe-ticks", "emit": { "onMessage": "TICKED", "failure": "FAILED" } }]] },
                            { "from": "idle", "to": "idle", "event": "TICKED", "effects": [
                                ["set", "@entity.ticks", ["array/concat", "@entity.ticks", ["@payload"]]]] },
                            { "from": "idle", "to": "idle", "event": "NAVIGATE", "effects": [
                                ["program/navigate", { "guest": "@entity.guest", "route": "@payload.route", "remount": ["default", "@payload.remount", false] }, emit]] },
                            { "from": "idle", "to": "idle", "event": "INPUTS", "effects": [
                                ["program/inputs", { "guest": "@entity.guest", "state": "@payload.orbital" }, emit]] },
                            { "from": "idle", "to": "idle", "event": "CLOSE", "effects": [
                                ["program/close", "@entity.guest", emit]] },
                            { "from": "idle", "to": "idle", "event": "ANSWERED", "effects": [
                                ["set", "@entity.answer", "@payload.result"],
                                ["set", "@entity.failed", null]] },
                            { "from": "idle", "to": "idle", "event": "FAILED", "effects": [
                                ["set", "@entity.failed", "@payload"],
                                ["set", "@entity.answer", null]] }
                        ]
                    }
                }]
            }]
        }))
        .map_err(|e| format!("opener program: {e}"))?;
        let host = Host::new(Arc::new(DriverAdapter::memory())).shared();
        host.register_schema(schema).map_err(|e| format!("opener register: {e}"))?;
        Ok(Self { host })
    }

    /// Dispatches `event` on the opener and reads back what the guest answered: `Ok(answer)` or `Err(refusal)`.
    fn run(&self, event: &str, payload: Json) -> Result<Json, String> {
        // Each call reads only what this event answered, never an earlier call's answer or refusal.
        self.host.dispatch("Opener", RUN_ENTITY, "RESET", orbital_core::Value::Null).map_err(|e| format!("opener RESET: {e}"))?;
        self.host.dispatch("Opener", RUN_ENTITY, event, value_of(payload)?).map_err(|e| format!("opener {event}: {e}"))?;
        let mut entity = orbital_core::json_value::value_to_json(&self.host.kernel().snapshot_trait("Opener", Some(RUN_ENTITY)).entity);
        let mut take = |key: &str| entity.get_mut(key).map(Json::take).filter(|v| !v.is_null());
        if let Some(refusal) = take("failed") {
            return Err(guest_error(&refusal));
        }
        take("answer").ok_or_else(|| format!("the opener did not settle after {event}"))
    }
}

struct Served {
    host: Arc<Host>,
    capture: Capture,
    traits: Vec<String>,
    orbital_by_trait: BTreeMap<String, String>,
}

fn served_open(schema: &OrbitalSchema, source: &Path, route: &str) -> Result<Served, String> {
    let host = boot(schema.clone(), source, Arc::new(DriverAdapter::memory()), &BootEnv::new(), false, HostConfig::default()).map_err(|e| e.to_string())?;
    let mut seeds = host.boot_mounts(ProtocolSide::Server);
    let circuit = host.kernel().page_circuit(None, Some(route))?;
    for seed in host.kernel().route_mounts(route) {
        if !seeds.iter().any(|held| held.r#trait == seed.r#trait) {
            seeds.push(seed);
        }
    }
    let seed_trait = seeds.first().map(|s| s.r#trait.clone()).unwrap_or_default();
    let payload = host.kernel().route_mount_payload(route);
    let outcomes = host.dispatch_mount(circuit.route.as_deref(), &seeds, RUN_ENTITY, payload).map_err(|e| e.to_string())?;
    Ok(Served {
        capture: served_capture(&outcomes, &seed_trait, "INIT"),
        traits: circuit.traits,
        orbital_by_trait: circuit.orbital_by_trait,
        host,
    })
}

/// Each store the mock seeder minted rows for that holds none after the mount: `(collection, minted, stored)`.
fn unwritten_seed(served: &Served, program: &Json) -> Vec<(String, usize, usize)> {
    let seeder = served.host.kernel().snapshot_trait(SEEDER_TRAIT, Some(RUN_ENTITY));
    let entity = orbital_core::json_value::value_to_json(&seeder.entity);
    let Some(stores) = entity.get("dsStores").and_then(Json::as_object) else { return Vec::new() };
    let mut entity_of: BTreeMap<String, String> = BTreeMap::new();
    for orbital in program["orbitals"].as_array().into_iter().flatten() {
        let owned = std::iter::once(&orbital["entity"]).chain(orbital["auxiliaryEntities"].as_array().into_iter().flatten());
        for e in owned {
            if let (Some(name), Some(collection)) = (e["name"].as_str(), e["collection"].as_str()) {
                entity_of.insert(collection.to_string(), name.to_string());
            }
        }
    }
    let source = served.host.kernel().entity_source();
    stores
        .iter()
        .filter_map(|(collection, rows)| {
            let minted = rows.as_array().map_or(0, Vec::len);
            let name = entity_of.get(collection)?;
            let stored = source.list(name).map(|r| r.len()).unwrap_or(0);
            (minted > 0 && stored == 0).then(|| (collection.clone(), minted, stored))
        })
        .collect()
}

fn served_event(served: &Served, trait_name: &str, event: &str) -> Result<Capture, String> {
    let outcomes = served.host.dispatch(trait_name, RUN_ENTITY, event, orbital_core::Value::Null).map_err(|e| e.to_string())?;
    Ok(served_capture(&outcomes, trait_name, event))
}

fn served_rows(served: &Served) -> Json {
    let rows: BTreeMap<String, Json> = served
        .traits
        .iter()
        .map(|name| {
            let snapshot = served.host.kernel().snapshot_trait(name, Some(RUN_ENTITY));
            // A trait with no instance yet holds no row, as the guest's `program/inputs` lists only instances.
            let entity = if snapshot.state.is_empty() {
                Json::Null
            } else {
                serde_json::to_value(&snapshot.entity).unwrap_or_else(|e| json!({ "unserializable": e.to_string() }))
            };
            (name.clone(), entity)
        })
        .collect();
    json!(rows)
}

struct Guest<'a> {
    opener: &'a Opener,
    handle: String,
    capture: Capture,
}

fn guest_open<'a>(opener: &'a Opener, program: &Json, route: &str) -> Result<Guest<'a>, String> {
    let answer = opener.run("OPEN", json!({ "program": program, "route": route }))?;
    let handle = answer.get("guest").and_then(Json::as_str).unwrap_or_default().to_string();
    let capture = guest_capture(&answer);
    Ok(Guest { opener, handle, capture })
}

fn guest_event(guest: &Guest<'_>, trait_name: &str, event: &str) -> Result<Capture, String> {
    let answer = guest.opener.run("CALL", json!({ "address": format!("{trait_name}.{event}"), "payload": null }))?;
    Ok(guest_capture(&answer))
}

fn guest_rows(guest: &Guest<'_>, served: &Served) -> Result<Json, String> {
    let orbitals: std::collections::BTreeSet<&String> = served.traits.iter().filter_map(|t| served.orbital_by_trait.get(t)).collect();
    let mut rows: BTreeMap<String, Json> = served.traits.iter().map(|t| (t.clone(), Json::Null)).collect();
    for orbital in orbitals {
        let answer = guest.opener.run("INPUTS", json!({ "orbital": orbital }))?;
        for instance in answer.pointer("/state/traits").and_then(Json::as_array).into_iter().flatten() {
            let Some(name) = instance.get("trait").and_then(Json::as_str) else { continue };
            if let Some(slot) = rows.get_mut(name) {
                if slot.is_null() {
                    *slot = instance.get("entity").cloned().unwrap_or(Json::Null);
                }
            }
        }
    }
    Ok(json!(rows))
}

fn fired_events(content: &Json, out: &mut Vec<String>) {
    match content {
        Json::Object(map) => {
            if let Some(props) = map.get("type").and_then(Json::as_str).and_then(|pattern| PATTERN_EVENT_NAME_PROPS.get(pattern)) {
                let mut props: Vec<_> = props.iter().collect();
                props.sort_by(|a, b| a.0.cmp(b.0));
                for (prop, (kind, field)) in props {
                    if !FIRING_KINDS.contains(kind) {
                        continue;
                    }
                    let Some(named) = map.get(*prop) else { continue };
                    match (*kind, named) {
                        ("event", Json::String(name)) => out.push(name.clone()),
                        ("event-list", Json::Array(items)) => out.extend(items.iter().filter_map(|item| item.get(*field).and_then(Json::as_str)).map(str::to_string)),
                        ("event-map", Json::Object(entries)) => out.extend(entries.values().filter_map(Json::as_str).map(str::to_string)),
                        _ => {}
                    }
                }
            }
            map.values().for_each(|child| fired_events(child, out));
        }
        Json::Array(items) => items.iter().for_each(|child| fired_events(child, out)),
        _ => {}
    }
}

fn controls(capture: &Capture) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for paint in capture.paints.as_array().into_iter().flatten() {
        let Some(trait_name) = paint.get("trait").and_then(Json::as_str) else { continue };
        let mut events = Vec::new();
        fired_events(&paint["content"], &mut events);
        for event in events {
            let pair = (trait_name.to_string(), event);
            if !found.contains(&pair) {
                found.push(pair);
            }
        }
    }
    found
}

/// The one rule that makes two sides comparable: numbers compare by value, because a served snapshot holds `1.0` where the
/// guest's JSON answer holds `1`. Both sides read the same frozen clock (`FROZEN_NOW`), so instants compare exactly.
fn same(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => x.as_f64() == y.as_f64(),
        (Json::Array(x), Json::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        (Json::Object(x), Json::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w))),
        _ => a == b,
    }
}

fn diff(a: &Json, b: &Json, at: String, out: &mut Vec<Json>) {
    match (a, b) {
        (Json::Object(x), Json::Object(y)) => {
            for key in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                let path = format!("{at}/{key}");
                match (x.get(key), y.get(key)) {
                    (Some(p), Some(q)) => diff(p, q, path, out),
                    (p, q) => out.push(json!({ "path": path, "served": p, "guest": q })),
                }
            }
        }
        (Json::Array(x), Json::Array(y)) => {
            if x.len() != y.len() {
                out.push(json!({ "path": format!("{at}#len"), "served": x.len(), "guest": y.len() }));
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                diff(p, q, format!("{at}/{i}"), out);
            }
            for (i, extra) in x.iter().enumerate().skip(y.len()) {
                out.push(json!({ "path": format!("{at}/{i}"), "served": extra, "guest": Json::Null }));
            }
            for (i, extra) in y.iter().enumerate().skip(x.len()) {
                out.push(json!({ "path": format!("{at}/{i}"), "served": Json::Null, "guest": extra }));
            }
        }
        _ if same(a, b) => {}
        _ => out.push(json!({ "path": at, "served": a, "guest": b })),
    }
}

const FROZEN_NOW: i64 = 1_781_481_600_000;

fn frozen_clock() -> i64 {
    FROZEN_NOW
}

struct Sweep {
    behavior: String,
    /// The open guest's handle, read as a placeholder in findings so they do not differ per session.
    handle: String,
    findings: Vec<Json>,
    notes: Vec<Json>,
}

impl Sweep {
    /// One finding per divergence, holding each differing path's two values rather than both sides whole.
    fn finding(&mut self, page: &str, step: &str, kind: &str, served: Json, guest: Json) {
        let mut guest = guest;
        mask_guest(&mut guest, &self.handle);
        if same(&served, &guest) {
            self.findings.push(json!({ "behavior": self.behavior, "page": page, "step": step, "kind": kind, "both": served }));
            return;
        }
        let mut diffs = Vec::new();
        diff(&served, &guest, String::new(), &mut diffs);
        self.findings.push(json!({ "behavior": self.behavior, "page": page, "step": step, "kind": kind, "diffs": diffs }));
    }

    fn note(&mut self, page: &str, step: &str, why: &str) {
        self.notes.push(json!({ "behavior": self.behavior, "page": page, "step": step, "kind": "unsupported", "served": why, "guest": why }));
    }

    fn compare(&mut self, page: &str, step: &str, served: &Result<Capture, String>, guest: &Result<Capture, String>) {
        match (served, guest) {
            (Ok(s), Ok(g)) => {
                for (kind, a, b) in [("paints", &s.paints, &g.paints), ("emitted", &s.emitted, &g.emitted), ("outcomes", &s.outcomes, &g.outcomes)] {
                    if !same(a, b) {
                        self.finding(page, step, kind, a.clone(), b.clone());
                    }
                }
            }
            (Err(a), Err(b)) if a == b || b.strip_prefix("mounting ").is_some_and(|rest| rest.ends_with(&format!(": {a}"))) => {}
            _ => {
                let kind = if step == "mount" { "open-failed" } else { "outcomes" };
                let side = |r: &Result<Capture, String>| match r {
                    Ok(c) => json!({ "paints": c.paints, "emitted": c.emitted, "outcomes": c.outcomes }),
                    Err(e) => json!({ "error": e }),
                };
                self.finding(page, step, kind, side(served), side(guest));
            }
        }
    }

    fn compare_rows(&mut self, page: &str, step: &str, served: &Json, guest: &Result<Json, String>) {
        match guest {
            Ok(rows) if same(served, rows) => {}
            Ok(rows) => self.finding(page, step, "rows", served.clone(), rows.clone()),
            Err(e) => self.finding(page, step, "rows", served.clone(), json!({ "error": e })),
        }
    }
}

fn route_pages(program: &Json) -> Vec<(String, String)> {
    let mut pages: Vec<(String, String)> = Vec::new();
    for orbital in program.get("orbitals").and_then(Json::as_array).into_iter().flatten() {
        for page in orbital.get("pages").and_then(Json::as_array).into_iter().flatten() {
            let routed = page.get("protocol").and_then(Json::as_str).map_or(true, |p| p == "route");
            let (Some(name), Some(path)) = (page.get("name").and_then(Json::as_str), page.get("path").and_then(Json::as_str)) else { continue };
            if routed && !pages.iter().any(|(_, held)| held == path) {
                pages.push((name.to_string(), path.to_string()));
            }
        }
    }
    pages
}

fn sweep_behavior(path: &Path, seed: &Result<Json, String>, grants: &Json) -> (Vec<Json>, Vec<Json>) {
    let mut sweep = Sweep { behavior: path.display().to_string(), handle: String::new(), findings: Vec::new(), notes: Vec::new() };
    let program = match resolve(path) {
        Ok(program) => program,
        Err(e) => {
            sweep.finding("", "resolve", "open-failed", json!({ "error": e }), json!({ "error": e }));
            return (sweep.findings, sweep.notes);
        }
    };
    let program = match serde_json::to_value(&program) {
        Ok(json) if holds_seed_marks(&json) => program,
        Ok(_) => match seed {
            Ok(seed) => match compose_with_seed(&program, seed) {
                Ok(composed) => composed,
                Err(e) => {
                    sweep.finding("", "compose", "compose-unavailable", json!({ "error": e }), json!({ "error": e }));
                    return (sweep.findings, sweep.notes);
                }
            },
            Err(e) => {
                sweep.finding("", "compose", "compose-unavailable", json!({ "error": e }), json!({ "error": e }));
                return (sweep.findings, sweep.notes);
            }
        },
        Err(e) => {
            sweep.finding("", "resolve", "open-failed", json!({ "error": e.to_string() }), json!({ "error": e.to_string() }));
            return (sweep.findings, sweep.notes);
        }
    };
    let program_json = match serde_json::to_value(&program) {
        Ok(json) => json,
        Err(e) => {
            sweep.finding("", "resolve", "open-failed", json!({ "error": e.to_string() }), json!({ "error": e.to_string() }));
            return (sweep.findings, sweep.notes);
        }
    };
    let held: orbital_core::runtime::os_grants::OsGrants = match serde_json::from_value(grants.clone()) {
        Ok(held) => held,
        Err(e) => {
            sweep.finding("", "opener", "open-failed", Json::Null, json!({ "error": e.to_string() }));
            return (sweep.findings, sweep.notes);
        }
    };
    let mut narrowed = program.clone();
    orbital_core::runtime::os_grants::narrow_program_grants(&mut narrowed, &held, &orbital_core::runtime::os_grants::OsGrants::default());
    let grant_view = |schema: &OrbitalSchema| {
        json!({ "app": schema.os_grants, "orbitals": schema.orbitals.iter().map(|o| json!({ "name": o.name, "osGrants": o.os_grants })).collect::<Vec<_>>() })
    };
    let (declared_view, narrowed_view) = (grant_view(&program), grant_view(&narrowed));
    if !same(&declared_view, &narrowed_view) {
        sweep.finding("", "grants", "launchpad-lacks", declared_view, narrowed_view);
    }
    let program = narrowed;
    let opener = match Opener::new(grants) {
        Ok(opener) => opener,
        Err(e) => {
            sweep.finding("", "opener", "open-failed", Json::Null, json!({ "error": e }));
            return (sweep.findings, sweep.notes);
        }
    };
    for (page_name, route) in route_pages(&program_json) {
        let page = format!("{page_name} {route}");
        let served = served_open(&program, path, &route);
        let guest = guest_open(&opener, &program_json, &route);
        let (served_mount, guest_mount) = (
            served.as_ref().map(|s| s.capture.clone()).map_err(String::clone),
            guest.as_ref().map(|g| g.capture.clone()).map_err(String::clone),
        );
        sweep.compare(&page, "mount", &served_mount, &guest_mount);
        // Parity hides a seed that fails the same way on both sides, so a failed seed is its own finding.
        for (side, capture) in [("served", &served_mount), ("guest", &guest_mount)] {
            let failed: Vec<Json> = capture
                .as_ref()
                .map(|c| c.emitted.as_array().into_iter().flatten().filter(|e| e["event"] == "DATASET_FAILED").cloned().collect())
                .unwrap_or_default();
            if !failed.is_empty() {
                sweep.findings.push(json!({ "behavior": sweep.behavior, "page": page, "step": "mount", "kind": "seed-failed", "side": side, "both": failed }));
            }
        }
        if let Ok(served) = &served {
            for (collection, minted, stored) in unwritten_seed(served, &program_json) {
                sweep.findings.push(json!({ "behavior": sweep.behavior, "page": page, "step": "mount", "kind": "seed-not-written", "both": { "collection": collection, "minted": minted, "stored": stored } }));
            }
        }
        let (Ok(served), Ok(guest)) = (served, guest) else { continue };
        sweep.handle = guest.handle.clone();
        sweep.compare_rows(&page, "mount", &served_rows(&served), &guest_rows(&guest, &served));
        for (trait_name, event) in controls(&served.capture) {
            let step = format!("{trait_name}.{event}");
            let declares_payload = served.host.kernel().event_payload_schema(&trait_name, &event).is_some_and(|fields| !fields.is_empty());
            if declares_payload {
                sweep.note(&page, &step, "no helper builds a sample payload from a declared event payload shape; the event is not fired");
                continue;
            }
            let (s, g) = (served_event(&served, &trait_name, &event), guest_event(&guest, &trait_name, &event));
            sweep.compare(&page, &step, &s, &g);
            sweep.compare_rows(&page, &step, &served_rows(&served), &guest_rows(&guest, &served));
        }
        let _ = opener.run("CLOSE", Json::Null);
    }
    (sweep.findings, sweep.notes)
}

fn shard() -> Result<(usize, usize), String> {
    let Ok(spec) = std::env::var("CATALOG_PARITY_SHARD") else { return Ok((0, 1)) };
    let (i, n) = spec.split_once('/').ok_or_else(|| format!("CATALOG_PARITY_SHARD={spec}: expected i/n"))?;
    let (i, n): (usize, usize) = (i.parse().map_err(|e| format!("shard index: {e}"))?, n.parse().map_err(|e| format!("shard count: {e}"))?);
    if n == 0 || i >= n {
        return Err(format!("CATALOG_PARITY_SHARD={spec}: the index must be below the count"));
    }
    Ok((i, n))
}

fn sweep_in_thread(path: PathBuf, seed: Result<Json, String>, grants: Json) -> (Vec<Json>, Vec<Json>) {
    let name = path.display().to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("catalog-parity".to_string()).stack_size(128 * 1024 * 1024).spawn(move || {
        let _ = tx.send(sweep_behavior(&path, &seed, &grants));
    });
    let failure = |step: &str, why: String| {
        (vec![json!({ "behavior": name, "page": "", "step": step, "kind": "open-failed", "served": { "error": why }, "guest": { "error": why } })], Vec::new())
    };
    if let Err(e) = spawned {
        return failure("spawn", e.to_string());
    }
    match rx.recv_timeout(BEHAVIOR_BUDGET) {
        Ok(report) => report,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => failure("timeout", format!("no answer within {} seconds", BEHAVIOR_BUDGET.as_secs())),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => failure("panic", "the sweep of this behavior panicked".to_string()),
    }
}

#[test]
fn the_catalog_guest_open_matches_the_served_registration() {
    orbital_core::time_util::install_clock(frozen_clock);
    let Ok(list) = std::env::var("CATALOG_PARITY_LIST") else {
        eprintln!("catalog_parity skipped: set CATALOG_PARITY_LIST to a newline list of behavior paths");
        return;
    };
    let (index, count) = shard().unwrap_or_else(|e| panic!("{e}"));
    let text = std::fs::read_to_string(&list).unwrap_or_else(|e| panic!("CATALOG_PARITY_LIST {list}: {e}"));
    let paths: Vec<PathBuf> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).enumerate().filter(|(i, _)| i % count == index).map(|(_, p)| p).collect();
    let seed = seed_program();
    let grants = match std::env::var("CATALOG_PARITY_GRANTS") {
        Ok(file) => match std::fs::read_to_string(&file) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| panic!("CATALOG_PARITY_GRANTS {file}: {e}")),
            Err(_) => {
                let resolved = launchpad_guest_grants().unwrap_or_else(|e| panic!("the launchpad's guest grants: {e}"));
                std::fs::write(&file, resolved.to_string()).unwrap_or_else(|e| panic!("CATALOG_PARITY_GRANTS {file}: {e}"));
                resolved
            }
        },
        Err(_) => launchpad_guest_grants().unwrap_or_else(|e| panic!("the launchpad's guest grants: {e}")),
    };
    let (mut findings, mut notes) = (Vec::new(), Vec::new());
    for path in paths {
        let (f, n) = sweep_in_thread(path, seed.clone(), grants.clone());
        findings.extend(f);
        notes.extend(n);
    }
    for line in notes.iter().chain(findings.iter()) {
        println!("{line}");
    }
    assert!(findings.is_empty(), "{} divergences between the served registration and the catalog guest open", findings.len());
}

/// A guest whose `BAD` emits an event missing its required field and whose `GOOD` paints.
fn strict_guest() -> Json {
    json!({
        "name": "Strict", "typeContractVersion": 1,
        "orbitals": [{
            "name": "S", "entity": { "name": "Cell", "persistence": "runtime", "fields": [{ "name": "id", "type": "string" }] },
            "traits": [{ "name": "T", "linkedEntity": "Cell",
                "emits": [{ "event": "OUT", "scope": "external", "payloadSchema": [{ "name": "itemId", "type": "string", "required": true }] }],
                "stateMachine": { "states": [{ "name": "idle", "isInitial": true }],
                    "transitions": [
                        { "from": "idle", "to": "idle", "event": "INIT", "effects": [["render-ui", "main", { "type": "typography", "content": "ready" }]] },
                        { "from": "idle", "to": "idle", "event": "BAD", "effects": [["emit", "OUT", {}]] },
                        { "from": "idle", "to": "idle", "event": "GOOD", "effects": [["render-ui", "main", { "type": "typography", "content": "good" }]] }
                    ] } }],
            "pages": [{ "name": "P", "path": "/", "traits": [{ "ref": "T" }] }]
        }]
    })
}

#[test]
fn a_failed_guest_step_does_not_fail_the_next_step_of_the_same_guest() {
    let opener = Opener::new(&json!({ "kernel": [{ "target": "guest", "kinds": ["spawn"] }] })).unwrap();
    opener.run("OPEN", json!({ "program": strict_guest(), "route": "/" })).unwrap();
    let bad = opener.run("CALL", json!({ "address": "T.BAD", "payload": null }));
    assert!(bad.is_err(), "the bad step is refused: {bad:?}");
    let good = opener.run("CALL", json!({ "address": "T.GOOD", "payload": null }));
    assert!(good.is_ok(), "the next step is answered, not refused by the earlier failure: {good:?}");
}

#[test]
fn control_a_fresh_guest_opened_after_a_failed_step_mounts() {
    let opener = Opener::new(&json!({ "kernel": [{ "target": "guest", "kinds": ["spawn"] }] })).unwrap();
    opener.run("OPEN", json!({ "program": strict_guest(), "route": "/" })).unwrap();
    let _ = opener.run("CALL", json!({ "address": "T.BAD", "payload": null }));
    let _ = opener.run("CLOSE", Json::Null);
    let reopened = opener.run("OPEN", json!({ "program": strict_guest(), "route": "/" }));
    assert!(reopened.is_ok(), "{reopened:?}");
}

/// Diagnostic, run by hand: boots the launchpad on a behavior route as `orb launchpad` serves it and prints what the
/// catalog data trait answered.
#[test]
#[ignore]
fn launchpad_catalog_listing_settles() {
    let path = workspace_root().join(LAUNCHPAD);
    let program = resolve(&path).unwrap();
    let served = served_open(&program, &path, "/almadar-behaviors/std-note").unwrap();
    for second in 0..2 {
        let snapshot = served.host.kernel().snapshot_trait("BehaviorCatalogData", Some(RUN_ENTITY));
        eprintln!("BehaviorCatalogData state at {second}s: {}", snapshot.state);
        if snapshot.state != "loading" {
            let entity = orbital_core::json_value::value_to_json(&snapshot.entity);
            for key in ["error", "lastError", "message", "total", "count"] {
                if let Some(v) = entity.get(key) {
                    eprintln!("  {key}: {}", v.to_string().chars().take(400).collect::<String>());
                }
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    let emitted: Vec<String> = served.capture.emitted.as_array().into_iter().flatten().filter_map(|e| e["event"].as_str().map(str::to_string)).collect();
    for name in ["PLAYGROUND_OPENED", "CATALOG_LOAD", "SCOPE", "LISTED", "LIST_FAILED", "APP_SHOWN", "BOARD_OPEN"] {
        eprintln!("{name}: {}", emitted.iter().filter(|e| e.as_str() == name).count());
    }
    let moved: Vec<String> = served.capture.outcomes.as_array().into_iter().flatten().filter(|o| o["trait"].as_str().is_some_and(|t| t.starts_with("Playground") || t == "LaunchpadStage" || t == "BehaviorCatalogData" || t == "LaunchpadBehaviorRoute")).map(|o| format!("{}.{} {}->{} fired={}", o["trait"].as_str().unwrap_or(""), o["event"].as_str().unwrap_or(""), o["from"], o["to"], o["fired"])).collect();
    eprintln!("moved: {moved:#?}");
    for e in served.capture.emitted.as_array().into_iter().flatten() {
        if e["event"].as_str().is_some_and(|n| n.contains("LIST") || n.contains("CATALOG") || n.contains("FAIL")) {
            eprintln!("{}", e.to_string().chars().take(600).collect::<String>());
        }
    }
}

/// Diagnostic, run by hand: the launchpad's browser kernel (as the wasm client runs it) opens the playground tab and
/// prints whether that dispatch posts a server leg, which is what lets the server's catalog trait list.
#[test]
#[ignore]
fn launchpad_playground_open_posts_a_server_leg() {
    let program = resolve(&workspace_root().join(LAUNCHPAD)).unwrap();
    let browser = orbital_core::embed::EmbeddedKernel::new(program, "c1").unwrap();
    let circuit = browser.page_circuit(None, Some("/")).unwrap();
    let mut traits: Vec<String> = circuit.mounts.iter().map(|s| s.r#trait.clone()).collect();
    traits.extend(circuit.embedded.iter().filter(|t| !traits.contains(t)).cloned().collect::<Vec<_>>());
    let entered = browser.enter(Some("/"), "c1\u{0}/", traits).unwrap();
    browser.dispatch_mount(&entered.seeds, None, orbital_core::Value::Null).unwrap();
    for name in ["LaunchpadStage", "BehaviorCatalogData", "PlaygroundFrame"] {
        eprintln!("{name}: mode {:?}, state {:?}", browser.dispatch_mode(name), browser.get_state(name, None));
    }
    let payload = value_of(json!({ "behavior": "almadar-std/std-behavior-playground", "label": "Behavior playground", "icon": "flask-conical", "topic": "dev" })).unwrap();
    let dispatched = browser.dispatch("LaunchpadStage", None, "TAB_OPEN", payload).unwrap();
    let wire = serde_json::to_value(&dispatched).unwrap();
    eprintln!("emitted: {:?}", dispatched.response.emitted_events.iter().map(|e| e.event.clone()).collect::<Vec<_>>());
    eprintln!("server legs: {}", dispatched.server_legs.len());
    eprintln!("legs: {}", wire["serverLegs"].to_string().chars().take(800).collect::<String>());
    eprintln!("stateful leg: {}", wire["statefulLeg"].to_string().chars().take(400).collect::<String>());
    eprintln!("BehaviorCatalogData after: {:?}", browser.get_state("BehaviorCatalogData", None));
}

/// G-STD-303 (EXEC-DIST-10 + EXEC-ENDPOINT-3a): the launchpad's deep link mounts `LaunchpadBehaviorRoute`, whose INIT
/// reaches the playground's catalog through `[client]` route traits the host leaves to the browser, so the browser
/// kernel posts the catalog's listing as a hop after the mount, or the catalog never lists.
#[test]
fn the_launchpad_behavior_deep_link_posts_the_catalog_listing_after_the_mount() {
    let program = resolve(&workspace_root().join(LAUNCHPAD)).unwrap();
    let browser = orbital_core::embed::EmbeddedKernel::new(program, "c1").unwrap();
    let route = "/almadar-std/std-browse";
    let circuit = browser.page_circuit(None, Some(route)).unwrap();
    let mut traits: Vec<String> = circuit.mounts.iter().map(|s| s.r#trait.clone()).collect();
    traits.extend(circuit.embedded.iter().filter(|t| !traits.contains(t)).cloned().collect::<Vec<_>>());
    let entered = browser.enter(Some(route), &format!("c1\u{0}{route}"), traits).unwrap();
    let payload = value_of(json!({ "package": "almadar-std", "name": "std-browse" })).unwrap();
    let mounted = browser.dispatch_route_mount(Some(route), &entered.seeds, None, payload).unwrap();
    let hops: Vec<(Option<String>, String)> = mounted.hops.iter().map(|h| (h.leg.target_trait.clone(), h.leg.event.clone())).collect();
    assert!(hops.contains(&(Some("BehaviorCatalogData".to_string()), "CATALOG_LOAD".to_string())), "{hops:?}");
}

#[test]
fn control_the_launchpad_home_mount_posts_no_hop() {
    let program = resolve(&workspace_root().join(LAUNCHPAD)).unwrap();
    let browser = orbital_core::embed::EmbeddedKernel::new(program, "c1").unwrap();
    let circuit = browser.page_circuit(None, Some("/")).unwrap();
    let mut traits: Vec<String> = circuit.mounts.iter().map(|s| s.r#trait.clone()).collect();
    traits.extend(circuit.embedded.iter().filter(|t| !traits.contains(t)).cloned().collect::<Vec<_>>());
    let entered = browser.enter(Some("/"), "c1\u{0}/", traits).unwrap();
    let mounted = browser.dispatch_route_mount(Some("/"), &entered.seeds, None, orbital_core::Value::Null).unwrap();
    assert!(mounted.hops.is_empty(), "{:?}", mounted.hops.iter().map(|h| (h.leg.target_trait.clone(), h.leg.event.clone())).collect::<Vec<_>>());
}

/// Diagnostic, run by hand: the launchpad's browser kernel mounting a behavior deep link, with the route params a
/// client mount carries, and what each seed emitted and moved.
#[test]
#[ignore]
fn launchpad_behavior_deep_link_mount() {
    let program = resolve(&workspace_root().join(LAUNCHPAD)).unwrap();
    let browser = orbital_core::embed::EmbeddedKernel::new(program, "c1").unwrap();
    let route = "/almadar-std/std-browse";
    let circuit = browser.page_circuit(None, Some(route)).unwrap();
    eprintln!("page: {:?}", circuit.page);
    let mut traits: Vec<String> = circuit.mounts.iter().map(|s| s.r#trait.clone()).collect();
    traits.extend(circuit.embedded.iter().filter(|t| !traits.contains(t)).cloned().collect::<Vec<_>>());
    let entered = browser.enter(Some(route), &format!("c1\u{0}{route}"), traits).unwrap();
    let payload = value_of(json!({ "package": "almadar-std", "name": "std-browse" })).unwrap();
    let mounted = browser.dispatch_route_mount(Some(route), &entered.seeds, None, payload).unwrap();
    let wire = serde_json::to_value(&mounted).unwrap();
    eprintln!("failed: {}", wire["failed"]);
    for seed in wire["seeds"].as_array().into_iter().flatten() {
        let t = seed["seed"]["trait"].as_str().unwrap_or("?");
        if !(t.starts_with("Launchpad") || t.starts_with("Playground") || t == "BehaviorCatalogData") { continue }
        let emitted: Vec<&str> = seed["response"]["emittedEvents"].as_array().into_iter().flatten().filter_map(|e| e["event"].as_str()).collect();
        eprintln!("{t}: emitted {emitted:?} error {}", seed["response"]["error"]);
        for routed in seed["serverLegs"].as_array().into_iter().flatten() {
            let l = &routed["leg"];
            eprintln!("  serverLeg {} -> {}.{} source {} traits {:?}", routed["orbital"], l["targetTrait"], l["event"], l["sourceTrait"], l["traits"].as_array().map(|ts| ts.iter().map(|t| t["trait"].clone()).collect::<Vec<_>>()));
        }
        eprintln!("  steps {:?}", seed["steps"].as_array().into_iter().flatten().map(|s| format!("{}.{}{}", s["trait"].as_str().or(s["traitName"].as_str()).unwrap_or("?"), s["event"].as_str().unwrap_or("?"), if s["delegated"] == true { "(delegated)" } else { "" })).collect::<Vec<_>>());
    }
    for leg in wire["legs"].as_array().into_iter().flatten() {
        let seeds: Vec<String> = leg["seeds"].as_array().into_iter().flatten().map(|s| format!("{}.{}", s["trait"].as_str().unwrap_or("?"), s["event"].as_str().unwrap_or("?"))).collect();
        eprintln!("leg {} route {} payload {} seeds {seeds:?}", leg["orbital"], leg["leg"]["route"], leg["leg"]["payload"]);
    }
    for name in ["LaunchpadStage", "BehaviorCatalogData", "LaunchpadBehaviorRoute", "PlaygroundRoute"] {
        eprintln!("{name}: {:?}", browser.get_state(name, None));
    }
    let Some(leg) = wire["legs"].as_array().and_then(|legs| legs.first()) else { return };
    let names: Vec<&str> = leg["seeds"].as_array().into_iter().flatten().filter_map(|s| s["trait"].as_str()).collect();
    let seeds: Vec<_> = entered.seeds.iter().filter(|s| names.contains(&s.r#trait.as_str())).cloned().collect();
    let schema = resolve(&workspace_root().join(LAUNCHPAD)).unwrap();
    let host = boot(schema, &workspace_root().join(LAUNCHPAD), Arc::new(DriverAdapter::memory()), &BootEnv::new(), false, HostConfig::default()).unwrap();
    let leg_payload = value_of(leg["leg"]["payload"].clone()).unwrap();
    let outcomes = host.dispatch_mount(Some(route), &seeds, RUN_ENTITY, leg_payload).unwrap();
    let moved: Vec<String> = outcomes.iter().map(|(t, r)| format!("{t}.{}{}", r.event.as_deref().unwrap_or("?"), if r.executed { "" } else { "(x)" })).collect();
    eprintln!("served moves: {moved:?}");
    for (t, r) in &outcomes {
        if !r.cascade_dropped.is_empty() || r.cascade_cut.is_some() {
            eprintln!("served cascade cut at {t}: dropped {:?} cut {:?}", r.cascade_dropped, r.cascade_cut);
        }
        if t == "LaunchpadBehaviorRoute" {
            eprintln!("served route step: payload {:?} failure {:?} effects {:?} consumed {:?}", r.payload, r.failure, r.effect_results, r.consumed_emits);
        }
    }
    for wait in [0u64, 2, 5] {
        std::thread::sleep(Duration::from_secs(wait));
        let data = host.kernel().snapshot_trait("BehaviorCatalogData", Some(RUN_ENTITY));
        let entity = orbital_core::json_value::value_to_json(&data.entity);
        eprintln!("served BehaviorCatalogData after +{wait}s: state {:?} entries {} error {}", data.state, entity["entries"].as_array().map_or(0, Vec::len), entity["error"]);
    }
}

/// Diagnostic, run by hand: opens each behavior of `CATALOG_SEED_PROBE` (comma-separated `.lolo` paths) as the catalog
/// does, on its first route page, and prints how many rows each entity holds after the mount.
#[test]
#[ignore]
fn catalog_guest_seeded_rows() {
    orbital_core::logging::init();
    let grants = launchpad_guest_grants().unwrap();
    let seed = seed_program().unwrap();
    for path in std::env::var("CATALOG_SEED_PROBE").unwrap().split(',') {
        let path = Path::new(path.trim());
        let program = resolve(path).unwrap();
        let json = serde_json::to_value(&program).unwrap();
        let marks = holds_seed_marks(&json);
        let composed = if marks || std::env::var("CATALOG_NO_SEED").is_ok() { program } else { compose_with_seed(&program, &seed).unwrap() };
        let program_json = serde_json::to_value(&composed).unwrap();
        let pages = route_pages(&program_json);
        let Some((name, route)) = pages.first().cloned() else { eprintln!("{}: no route page", path.display()); continue };
        let opener = Opener::new(&grants).unwrap();
        let opened = opener.run("OPEN", json!({ "program": program_json, "route": route }));
        eprintln!("== {} (declares marks: {marks}) page {name} {route}: open {}", path.display(), if opened.is_ok() { "ok".to_string() } else { format!("{opened:?}") });
        if let Ok(answer) = &opened {
            let seeder_moves: Vec<String> = answer["outcomes"].as_array().into_iter().flatten().filter(|o| o["trait"] == SEEDER_TRAIT).map(|o| format!("{}{}", o["event"].as_str().unwrap_or(""), if o["fired"] == true { "" } else { "(not fired)" })).collect();
            if let Ok(watch) = std::env::var("CATALOG_WATCH") {
                for o in program_json["orbitals"].as_array().into_iter().flatten() {
                    for t in o["traits"].as_array().into_iter().flatten().filter(|t| t["name"] == watch.as_str()) {
                        eprintln!("  resolved {watch} listens {} events {}", t["listens"], t["stateMachine"]["events"].to_string().chars().take(400).collect::<String>());
                    }
                    for t in o["traits"].as_array().into_iter().flatten().filter(|t| t["name"] == "HomeFigureFetcher") {
                        eprintln!("  resolved HomeFigureFetcher id {} emitIds {}", t["id"], t["emitIds"]);
                    }
                }
                let seen: Vec<String> = answer["outcomes"].as_array().into_iter().flatten().filter(|o| o["trait"] == watch.as_str()).map(|o| format!("{}:{}->{}{}", o["event"].as_str().unwrap_or(""), o["from"].as_str().unwrap_or(""), o["to"].as_str().unwrap_or(""), if o["fired"] == true { "" } else { "(not fired)" })).collect();
                eprintln!("  open outcomes of {watch}: {seen:?}");
            }
            let n = seeder_moves.len();
            eprintln!("  seeder moves ({n}): ...{:?}", &seeder_moves[n.saturating_sub(12)..]);
            for e in answer["emitted"].as_array().into_iter().flatten() {
                let ev = e["event"].as_str().unwrap_or("");
                if ev.contains("FAIL") || ev.contains("REFUSED") || ev.starts_with("DATASET") || ev.starts_with("PERSONA") {
                    eprintln!("  emitted {ev}: {}", e["payload"].to_string().chars().take(300).collect::<String>());
                }
            }
        }
        if let Ok(address) = std::env::var("CATALOG_CALL") {
            let call_payload: Json = std::env::var("CATALOG_CALL_PAYLOAD").ok().and_then(|p| serde_json::from_str(&p).ok()).unwrap_or_else(|| json!({}));
            match opener.run("CALL", json!({ "address": address, "payload": call_payload })) {
                Ok(a) => {
                    let moves: Vec<String> = a["outcomes"].as_array().into_iter().flatten().map(|o| format!("{}.{}{}", o["trait"].as_str().unwrap_or(""), o["event"].as_str().unwrap_or(""), if o["fired"] == true { "" } else { "(not fired)" })).collect();
                    eprintln!("  call moves {moves:?}");
                    eprintln!("  call emitted {}", a["emitted"].to_string().chars().take(900).collect::<String>());
                    for e in a["emitted"].as_array().into_iter().flatten() {
                        eprintln!("  call emitted {} keys {:?} totalCount {}", e["event"], e["payload"].as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()), e["payload"]["totalCount"]);
                    }
                }
                Err(e) => eprintln!("  call refused: {e}"),
            }
        }
        if let Ok(ops) = std::env::var("CATALOG_WRITE_OPS") {
            let ops: Json = serde_json::from_str(&ops).unwrap();
            match opener.run("CALL", json!({ "address": format!("{SEEDER_TRAIT}.DS_WRITE_CHUNK"), "payload": { "ops": ops } })) {
                Ok(a) => eprintln!("  write emitted {}", a["emitted"].to_string().chars().take(1200).collect::<String>()),
                Err(e) => eprintln!("  write refused: {e}"),
            }
        }
        if let Ok(address) = std::env::var("CATALOG_TICK_STREAM") {
            let started = opener.run("LIVE", json!({}));
            eprintln!("  live: {}", started.as_ref().map(|a| a.to_string()).unwrap_or_else(|e| e.clone()).chars().take(300).collect::<String>());
            if std::env::var("CATALOG_TICK_REMOUNT").is_ok() {
                let remounted = opener.run("NAVIGATE", json!({ "route": "/", "remount": true }));
                eprintln!("  remount: {}", remounted.as_ref().map(|a| a["page"].to_string()).unwrap_or_else(|e| e.clone()).chars().take(200).collect::<String>());
            }
            let called = opener.run("CALL", json!({ "address": address, "payload": {} }));
            eprintln!("  call {address}: {}", called.as_ref().map(|a| a["outcomes"].to_string()).unwrap_or_else(|e| e.clone()).chars().take(600).collect::<String>());
            std::thread::sleep(Duration::from_secs(3));
            let entity = orbital_core::json_value::value_to_json(&opener.host.kernel().snapshot_trait("Opener", Some(RUN_ENTITY)).entity);
            let ticks = entity["ticks"].as_array().cloned().unwrap_or_default();
            eprintln!("  tick items after 3s: {}", ticks.len());
            let painting: Vec<&Json> = ticks.iter().filter(|t| t["rendered"].as_array().is_some_and(|r| !r.is_empty())).collect();
            eprintln!("  items that painted: {}", painting.len());
            for t in painting.iter().take(3) {
                let painted: Vec<String> = t["rendered"].as_array().into_iter().flatten().map(|p| format!("{}: {}", p["trait"].as_str().unwrap_or(""), p["content"].to_string().chars().take(160).collect::<String>())).collect();
                eprintln!("    painted {painted:?}");
            }
            let executed: usize = ticks.iter().filter(|t| t["outcomes"].as_array().into_iter().flatten().any(|o| o["fired"] == true)).count();
            eprintln!("  items with a fired outcome: {executed}");
            for t in ticks.iter().take(3) {
                let traits: Vec<String> = t["outcomes"].as_array().into_iter().flatten().map(|o| format!("{}.{}", o["trait"].as_str().unwrap_or(""), o["event"].as_str().unwrap_or(""))).collect();
                let painted: Vec<&str> = t["rendered"].as_array().into_iter().flatten().filter_map(|p| p["trait"].as_str()).collect();
                eprintln!("    moved {traits:?} painted {painted:?}");
            }
            eprintln!("  failed: {}", entity["failed"]);
        }
        if let Ok(route) = std::env::var("CATALOG_NAVIGATE") {
            let started = std::time::Instant::now();
            match opener.run("NAVIGATE", json!({ "route": route })) {
                Ok(a) => {
                    let rendered = a["rendered"].as_array().map_or(0, Vec::len);
                    eprintln!("  navigate {route}: {:?} route {} page {} rendered {rendered} bytes {} outcomes {}", started.elapsed(), a["route"], a["page"], a.to_string().len(), a["outcomes"].as_array().map_or(0, Vec::len));
                    eprintln!("  navigate parts {:?}", a.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.to_string().len())).collect::<Vec<_>>()));
                    let mut moves: Vec<(usize, String)> = a["outcomes"].as_array().into_iter().flatten().map(|o| (o.to_string().len(), format!("{}.{} keys {:?}", o["trait"].as_str().unwrap_or("?"), o["event"].as_str().unwrap_or("?"), o.as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.to_string().len())).filter(|(_, n)| *n > 10_000).collect::<Vec<_>>())))).collect();
                    moves.sort_by(|x, y| y.0.cmp(&x.0));
                    eprintln!("  largest outcomes {:?}", &moves[..moves.len().min(8)]);
                    if let Ok(dump) = std::env::var("CATALOG_DUMP_PAINT") {
                        for p in a["rendered"].as_array().into_iter().flatten().filter(|p| p["trait"] == dump.as_str()) {
                            eprintln!("  paint {dump}: {p}");
                        }
                    }
                    let big: Vec<(String, usize)> = a["emitted"].as_array().into_iter().flatten().map(|e| (e["event"].as_str().unwrap_or("?").to_string(), e.to_string().len())).filter(|(_, n)| *n > 100_000).collect();
                    eprintln!("  big emits ({}) {big:?}", big.len());
                    for e in a["emitted"].as_array().into_iter().flatten().filter(|e| e.to_string().len() > 100_000).take(2) {
                        eprintln!("  big emit keys {:?} source {:?}", e.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()), e.get("source").or(e.get("trait")));
                    }
                    let mut sizes: Vec<(usize, String)> = a["rendered"].as_array().into_iter().flatten().map(|p| (p.to_string().len(), format!("{}@{}", p["trait"].as_str().unwrap_or("?"), p["slot"].as_str().unwrap_or("?")))).collect();
                    sizes.sort_by(|x, y| y.0.cmp(&x.0));
                    eprintln!("  largest paints {:?}", &sizes[..sizes.len().min(10)]);
                }
                Err(e) => eprintln!("  navigate {route} refused after {:?}: {e}", started.elapsed()),
            }
        }
        if let Ok(spec) = std::env::var("CATALOG_ACT_AS") {
            let (address, persona) = spec.split_once('=').unwrap();
            let answer = opener.run("CALL", json!({ "address": address, "payload": { "persona": persona } }));
            match answer {
                Ok(a) => {
                    let moves: Vec<String> = a["outcomes"].as_array().into_iter().flatten().map(|o| format!("{}.{}{}", o["trait"].as_str().unwrap_or(""), o["event"].as_str().unwrap_or(""), if o["fired"] == true { "" } else { "(x)" })).collect();
                    eprintln!("  act-as moves {moves:?}");
                    eprintln!("  act-as emitted {}", a["emitted"].to_string().chars().take(800).collect::<String>());
                }
                Err(e) => eprintln!("  act-as refused: {e}"),
            }
        }
        for second in [1u64, 3, 6] {
            std::thread::sleep(std::time::Duration::from_secs(if second == 1 { 1 } else { 3 }));
            if let Ok(answer) = opener.run("INPUTS", json!({ "orbital": "MockData" })) {
                for t in answer.pointer("/state/traits").and_then(Json::as_array).into_iter().flatten().filter(|t| t["trait"] == SEEDER_TRAIT) {
                    let e = &t["entity"];
                    eprintln!("  after {second}s: seeder mode {} active {} writes left {} sessions {}", e["dsMode"], e["dsActive"], e["dsWrites"].as_array().map_or(0, Vec::len), e["dsSessions"]);
                    if second == 1 && std::env::var("CATALOG_REPLAY_FIRST").is_ok() {
                        if let Some(first) = e["dsWrites"].as_array().and_then(|w| w.first()) {
                            let labels: Vec<String> = first["ops"].as_array().into_iter().flatten().map(|op| op[1].as_str().unwrap_or("").to_string()).collect();
                            eprintln!("    first chunk owner {} ops {} labels {labels:?}", first["owner"], labels.len());
                            match opener.run("CALL", json!({ "address": format!("{SEEDER_TRAIT}.DS_WRITE_CHUNK"), "payload": { "ops": first["ops"] } })) {
                                Ok(a) => eprintln!("    replay emitted {}", a["emitted"].to_string().chars().take(900).collect::<String>()),
                                Err(e) => eprintln!("    replay refused: {e}"),
                            }
                        }
                    }
                    if let Some(map) = e.as_object() {
                        let small: Vec<String> = map.iter().filter(|(k, v)| (k.starts_with("ds") || k.starts_with("ac")) && !["dsStores", "dsRegsAll", "dsWrites", "dsWriteAll", "dsStamps", "dsStamped", "dsOlinks"].contains(&k.as_str()) && v.to_string().len() < 200).map(|(k, v)| format!("{k}={v}")).collect();
                        eprintln!("    {}", small.join(" "));
                        for k in ["dsWriteAll", "acQueue", "acAnswers"] {
                            eprintln!("    {k} len {} first {}", e[k].as_array().map_or(0, Vec::len), e[k].as_array().and_then(|a| a.first()).map(|v| v.to_string().chars().take(200).collect::<String>()).unwrap_or_default());
                        }
                    }
                }
            }
        }
        if let Ok(spec) = std::env::var("CATALOG_SIGN_IN_AFTER") {
            let (address, persona) = spec.split_once('=').unwrap();
            let signed = opener.run("CALL", json!({ "address": address, "payload": { "persona": persona } }));
            let signed_events: Vec<String> = signed.as_ref().map(|a| a["emitted"].as_array().into_iter().flatten().filter_map(|e| e["event"].as_str().map(str::to_string)).collect()).unwrap_or_default();
            eprintln!("  signed in after seeding: {signed_events:?} {}", signed.as_ref().err().cloned().unwrap_or_default());
            let reloaded = opener.run("CALL", json!({ "address": "INIT", "payload": null }));
            eprintln!("  reload: {}", reloaded.as_ref().map(|_| "ok".to_string()).unwrap_or_else(|e| e.clone()));
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        let orbitals: Vec<String> = program_json["orbitals"].as_array().into_iter().flatten().filter_map(|o| o["name"].as_str().map(str::to_string)).collect();
        for orbital in orbitals {
            match opener.run("INPUTS", json!({ "orbital": orbital })) {
                Ok(answer) => {
                    eprintln!("  {orbital}: keys {:?} state keys {:?}", answer.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()), answer.get("state").and_then(Json::as_object).map(|o| o.keys().cloned().collect::<Vec<_>>()));
                    for t in answer.pointer("/state/traits").and_then(Json::as_array).into_iter().flatten() {
                        let full = t["trait"].as_str() == Some("MockDataMockSeed");
                        let text = t["entity"].to_string();
                        if full {
                            let stores: Vec<String> = t["entity"]["dsStores"].as_object().map(|o| o.iter().map(|(k, v)| format!("{k}:{}", v.as_array().map_or(0, Vec::len))).collect()).unwrap_or_default();
                            eprintln!("    seeder stores {stores:?}");
                            let e = &t["entity"];
                            eprintln!("    seeder mode {} active {} minted {} sessions {} error {}", e["dsMode"], e["dsActive"], e["dsMinted"], e["dsSessions"], e["dsError"]);
                            eprintln!("    seeder persona {}/{} writes left {}", e["dsPersonaOrbital"], e["dsPersonaTrait"], e["dsWrites"].as_array().map_or(0, Vec::len));
                        } else {
                            let watched = std::env::var("CATALOG_WATCH").ok().is_some_and(|w| t["trait"] == w.as_str());
                            eprintln!("    trait {} state {} entity {}", t["trait"], t["state"], text.chars().take(if watched { 4000 } else { 160 }).collect::<String>());
                        }
                    }
                }
                Err(e) => eprintln!("  {orbital}: inputs failed {e}"),
            }
        }
    }
}

/// A guest whose INIT makes the mock seeder's kind of call: a keyed, wired self `program/dispatch` into another trait
/// in a `seed-x` session. `DONE`/`FAIL` record how it settled.
fn self_dispatch_guest() -> Json {
    json!({
        "name": "SelfDispatch", "typeContractVersion": 1,
        "orbitals": [{
            "name": "O", "entity": { "name": "Cell", "persistence": "runtime", "fields": [{ "name": "id", "type": "string" }, { "name": "settled", "type": "string" }] },
            "traits": [
                { "name": "A", "linkedEntity": "Cell", "stateMachine": { "states": [{ "name": "idle", "isInitial": true }], "transitions": [
                    { "from": "idle", "to": "idle", "event": "INIT", "effects": [
                        ["program/dispatch", "O.B.PING", {}, { "orbital": "O", "targetTrait": "B", "event": "PING", "clientId": "seed-x", "wire": true }, { "key": "k", "emit": { "success": "DONE", "failure": "FAIL" } }]] },
                    { "from": "idle", "to": "idle", "event": "DONE", "effects": [["set", "@entity.settled", "done"]] },
                    { "from": "idle", "to": "idle", "event": "FAIL", "effects": [["set", "@entity.settled", ["str/concat", "fail ", ["json/stringify", "@payload"]]]] }
                ] } },
                { "name": "B", "linkedEntity": "Cell", "stateMachine": { "states": [{ "name": "idle", "isInitial": true }], "transitions": [
                    { "from": "idle", "to": "idle", "event": "PING", "effects": [["os/getenv", "HOME", { "emit": { "success": "GOT", "failure": "MISSED" } }]] },
                    { "from": "idle", "to": "idle", "event": "GOT", "effects": [["emit", "PONG", {}]] },
                    { "from": "idle", "to": "idle", "event": "MISSED", "effects": [["emit", "PONG", {}]] }
                ] } }
            ],
            "osGrants": { "env": ["HOME"] },
            "pages": [{ "name": "P", "path": "/", "traits": [{ "ref": "A" }] }]
        }]
    })
}

#[test]
#[ignore]
fn a_guest_keyed_self_dispatch_settles() {
    let opener = Opener::new(&json!({ "kernel": [{ "target": "guest", "kinds": ["spawn"] }], "env": ["HOME"] })).unwrap();
    let program = match std::env::var("SELF_DISPATCH_LOLO") {
        Ok(path) => serde_json::to_value(resolve(Path::new(&path)).unwrap()).unwrap(),
        Err(_) => self_dispatch_guest(),
    };
    let opened = opener.run("OPEN", json!({ "program": program, "route": "/" }));
    eprintln!("open: {}", opened.as_ref().map(|a| a["emitted"].to_string()).unwrap_or_else(|e| e.clone()));
    for second in 0..5 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let state = opener.run("INPUTS", json!({ "orbital": "O" })).unwrap();
        let settled: Vec<String> = state.pointer("/state/traits").and_then(Json::as_array).into_iter().flatten().map(|t| format!("{}={}", t["trait"], t["entity"]["settled"])).collect();
        eprintln!("after {}s: {settled:?}", second + 1);
    }
}

/// Diagnostic, run by hand (G-STD-306): how long the catalog's `behavior/resolve` takes on project-friday, parse and
/// resolve timed apart, through the resolve the program host runs.
#[test]
#[ignore]
fn project_friday_resolve_timing() {
    let path = workspace_root().join("packages/almadar-behaviors/behaviors/lolo/project-friday/organisms/project-friday.lolo");
    let compiler = compiler_options().unwrap();
    let started = std::time::Instant::now();
    let schema = orbital_compiler::phases::parse_file(&path).unwrap();
    let parsed = started.elapsed();
    let resolved = orbital_compiler::phases::lazy::resolve_program_with_path(schema, &compiler, Some(&path));
    let total = started.elapsed();
    let (schema, _) = resolved.unwrap();
    let traits: usize = schema.orbitals.iter().map(|o| o.traits.len()).sum();
    eprintln!("project-friday: parse {parsed:?}, resolve {:?}, total {total:?}; {} orbitals, {traits} traits", total - parsed, schema.orbitals.len());
}

