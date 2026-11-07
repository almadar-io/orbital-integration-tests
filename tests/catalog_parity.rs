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
                    { "name": "answer", "type": "object" }, { "name": "failed", "type": "object" }] },
                "traits": [{
                    "name": "Opener", "linkedEntity": "Box",
                    "stateMachine": {
                        "states": [{ "name": "idle", "isInitial": true }],
                        "transitions": [
                            { "from": "idle", "to": "idle", "event": "OPEN", "effects": [
                                ["program/open", "@payload.program", { "route": "@payload.route", "declared": true }, { "emit": { "success": "OPENED", "failure": "FAILED" } }]] },
                            { "from": "idle", "to": "idle", "event": "OPENED", "effects": [
                                ["set", "@entity.guest", "@payload.result.guest"],
                                ["set", "@entity.answer", "@payload.result"],
                                ["set", "@entity.failed", null]] },
                            { "from": "idle", "to": "idle", "event": "CALL", "effects": [
                                ["program/dispatch", "@payload.address", "@payload.payload", { "guest": "@entity.guest", "emit": emit["emit"] }]] },
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
        self.host.dispatch("Opener", RUN_ENTITY, event, value_of(payload)?).map_err(|e| e.to_string())?;
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
