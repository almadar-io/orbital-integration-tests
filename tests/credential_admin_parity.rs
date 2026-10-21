//! Parity: every session the `credentials` provider recorded (`fixtures/credentials/*.json`, from
//! `@almadar/integrations`) replays through the kernel's credential effects against a scripted host that answers
//! only the raw store, mode-flag and environment asks. The one deliberate gap: a service row's `configured`,
//! `mocked`, `status` and `needs` followed the retired integrations manager's factory, which the kernel has no
//! counterpart for; they are compared for the services the kernel lists, by id, service and backend only.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use orbital_core::credential_admin::{
    self, AdminAsks, Catalog, CredentialAdminOp, Refusal, RotateCounts, StoreListing, StoredName,
};
use orbital_core::effects::ServiceCaller;
use orbital_core::secrets::program_credentials;
use orbital_core::Value;
use serde_json::{json, Value as Json};

const LAST_STEP_ACTIONS: [&str; 6] = ["list", "set", "remove", "test", "mode", "rotate"];

struct Host {
    env: BTreeMap<String, String>,
    store: Option<RefCell<Vec<(String, String, String)>>>,
    enabled: bool,
    mode: RefCell<Option<String>>,
    refuse_mock: bool,
}

impl Host {
    fn of(fixture: &Json) -> Self {
        let setup = &fixture["setup"];
        let mut env: BTreeMap<String, String> = BTreeMap::new();
        for source in [&fixture["env"], &setup["processEnv"]] {
            for (name, value) in source.as_object().into_iter().flatten() {
                env.insert(name.clone(), value.as_str().expect("env value").to_string());
            }
        }
        let store = setup["store"].as_object().map(|store| {
            let enabled = store["enabled"].as_bool().expect("enabled");
            let rows = if enabled {
                store["entries"]
                    .as_array()
                    .expect("entries")
                    .iter()
                    .map(|e| (e["service"].as_str().unwrap().to_string(), e["envVar"].as_str().unwrap().to_string(), e["value"].as_str().unwrap().to_string()))
                    .collect()
            } else {
                Vec::new()
            };
            (enabled, RefCell::new(rows))
        });
        let (mode, refuse_mock) = match setup["modeControl"].as_object() {
            Some(control) => (Some(control["mode"].as_str().unwrap().to_string()), control["refuseMock"].as_bool().unwrap()),
            None => (None, false),
        };
        Host {
            env,
            enabled: store.as_ref().is_some_and(|(enabled, _)| *enabled),
            store: store.map(|(_, rows)| rows),
            mode: RefCell::new(mode),
            refuse_mock,
        }
    }
}

impl AdminAsks for Host {
    fn secret(&self, name: &str, _principal: Option<&str>) -> Result<Option<String>, String> {
        Ok(self.env.get(name).cloned())
    }
    fn store_list(&self) -> Result<Option<StoreListing>, String> {
        Ok(self.store.as_ref().map(|rows| StoreListing {
            enabled: self.enabled,
            entries: rows.borrow().iter().map(|(service, name, _)| StoredName { service: service.clone(), name: name.clone() }).collect(),
        }))
    }
    fn store_read(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.store.as_ref().and_then(|rows| rows.borrow().iter().find(|(_, n, _)| n == name).map(|(_, _, v)| v.clone())))
    }
    fn store_write(&self, service: &str, name: &str, value: &str) -> Result<(), String> {
        if !self.enabled {
            return Err("ALMADAR_CREDENTIAL_MASTER_KEY is not configured \u{2014} cannot store credentials".into());
        }
        let rows = self.store.as_ref().expect("a store");
        rows.borrow_mut().retain(|(_, n, _)| n != name);
        rows.borrow_mut().push((service.into(), name.into(), value.into()));
        Ok(())
    }
    fn store_remove(&self, name: &str) -> Result<bool, String> {
        let rows = self.store.as_ref().expect("a store");
        let before = rows.borrow().len();
        rows.borrow_mut().retain(|(_, n, _)| n != name);
        Ok(rows.borrow().len() != before)
    }
    fn store_rotate(&self) -> Result<RotateCounts, String> {
        if !self.enabled {
            return Err("ALMADAR_CREDENTIAL_MASTER_KEY is not configured \u{2014} cannot rotate".into());
        }
        Ok(RotateCounts { rotated: 0, already_current: self.store.as_ref().expect("a store").borrow().len() as u64, unreadable: 0 })
    }
    fn mode_read(&self) -> Result<Option<String>, String> {
        Ok(self.mode.borrow().clone())
    }
    fn mode_write(&self, mode: &str) -> Result<String, String> {
        if self.mode.borrow().is_none() {
            return Err(credential_admin::NO_MANAGER.to_string());
        }
        if mode == "mock" && self.refuse_mock {
            return Err("Mocked integrations are not permitted when NODE_ENV=production.".into());
        }
        *self.mode.borrow_mut() = Some(mode.to_string());
        Ok(mode.to_string())
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures() -> Vec<Json> {
    let dir = root().join("fixtures/credentials");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files.iter().map(|p| serde_json::from_str(&std::fs::read_to_string(p).expect("fixture")).expect("json")).collect()
}

fn catalog_of(setup: &Json) -> Catalog {
    let registry = std::fs::read_to_string(root().join("fixtures/credential-admin-registry.json")).expect("registry snapshot");
    let strings = |key: &str| -> Vec<String> { setup[key].as_array().expect(key).iter().map(|v| v.as_str().unwrap().to_string()).collect() };
    let catalog = Catalog::from_registry(&registry).expect("registry").ordered(strings("services"), strings("declaredOrder"));
    match setup.get("program") {
        Some(program) => catalog.with_program_credentials(&program_credentials(program)),
        None => catalog,
    }
}

fn to_json(value: &Value) -> Json {
    serde_json::to_value(value).expect("value")
}

fn comparable(mut data: Json) -> Json {
    if let Some(rows) = data.get_mut("services").and_then(Json::as_array_mut) {
        for row in rows {
            let object = row.as_object_mut().expect("service row");
            for gap in ["configured", "mocked", "status", "needs"] {
                object.remove(gap);
            }
        }
    }
    data
}

fn numbers_alike(value: Json) -> Json {
    match value {
        Json::Number(n) => json!(n.as_f64().expect("number")),
        Json::Array(items) => Json::Array(items.into_iter().map(numbers_alike).collect()),
        Json::Object(fields) => Json::Object(fields.into_iter().map(|(k, v)| (k, numbers_alike(v))).collect()),
        other => other,
    }
}

fn expected_message(message: &str) -> String {
    let mut out = message.to_string();
    for action in LAST_STEP_ACTIONS {
        out = out.replace(&format!("credentials.{action}"), &format!("credential-{action}"));
    }
    out
}

fn replay(fixture: &Json) -> usize {
    let case = fixture["case"].as_str().expect("case");
    let host = Host::of(fixture);
    let catalog = catalog_of(&fixture["setup"]);
    let mut replayed = 0;
    for step in fixture["steps"].as_array().expect("steps") {
        let action = step["action"].as_str().expect("action");
        let Some(op) = CredentialAdminOp::from_name(&format!("credential-{action}")) else {
            assert!(!LAST_STEP_ACTIONS.contains(&action), "{case}: {action}");
            assert_eq!(step["outcome"]["code"], "VALIDATION_ERROR", "{case}: an unknown action was a contract failure");
            continue;
        };
        let request: Value = serde_json::from_value(step["params"].clone()).expect("params");
        let caller = ServiceCaller { principal: "caller".into(), role: step["role"].as_str().map(str::to_string) };
        let got: Result<Value, Refusal> = credential_admin::run(&host, op, &request, Some(&caller), &|| Ok(catalog.clone()));
        let outcome = &step["outcome"];
        if step["params"]["service"] == "database" && matches!(action, "set" | "remove") {
            let refusal = got.expect_err("the `database` connection-ref exception is removed (owner ruling): a name must be declared");
            assert_eq!(refusal.message, "Service \"database\" declares no credentials", "{case} {action}");
            if action == "set" && outcome["ok"] == true {
                let (name, value) = (step["params"]["envVar"].as_str().unwrap(), step["params"]["value"].as_str().unwrap());
                host.store_write("database", name, value).expect("seed the row the retired exception would have stored");
            }
            replayed += 1;
            continue;
        }
        if outcome["ok"].as_bool().expect("ok") {
            let data = got.unwrap_or_else(|r| panic!("{case} {action}: refused {r:?}"));
            assert_eq!(
                comparable(numbers_alike(to_json(&data))),
                comparable(numbers_alike(outcome["data"].clone())),
                "{case} {action} {}",
                step["params"]
            );
        } else {
            let refusal = got.expect_err(&format!("{case} {action}: expected a refusal"));
            assert_eq!(refusal.message, expected_message(outcome["message"].as_str().expect("message")), "{case} {action}");
            assert_eq!(refusal.code, outcome["code"].as_str().map(|_| outcome["code"].as_str().unwrap()), "{case} {action}");
        }
        replayed += 1;
    }
    replayed
}

#[test]
fn every_recorded_credentials_session_replays_through_the_kernel_effects() {
    let all = fixtures();
    assert_eq!(all.len(), 20);
    let replayed: usize = all.iter().map(replay).sum();
    assert!(replayed >= 60, "replayed {replayed} steps");
}

#[test]
fn control_a_scripted_role_the_gate_refuses_still_changes_nothing() {
    let all = fixtures();
    let gate = all.iter().find(|f| f["case"] == "role-gate").expect("role-gate");
    let host = Host::of(gate);
    let catalog = catalog_of(&gate["setup"]);
    let request: Value = serde_json::from_value(json!({ "service": "apikeys", "envVar": "ALMADAR_API_KEY_SALT", "value": "v" })).unwrap();
    let viewer = ServiceCaller { principal: "p".into(), role: Some("viewer".into()) };
    let refused = credential_admin::run(&host, CredentialAdminOp::Set, &request, Some(&viewer), &|| Ok(catalog.clone())).unwrap_err();
    assert_eq!(refused.code, Some("AUTH_ERROR"));
    assert!(host.store.as_ref().unwrap().borrow().is_empty());
}

#[test]
fn control_the_catalog_is_not_widened_by_a_service_the_declared_order_leaves_out() {
    let all = fixtures();
    let set = all.iter().find(|f| f["case"] == "set-and-remove").expect("set-and-remove");
    let catalog = catalog_of(&set["setup"]);
    assert!(catalog.declared_for("queue").is_empty());
    assert_eq!(catalog.declared_for("apikeys").len(), 1);
}

/// The shared `kv-trace.json` (the Node and browser kv ports replay it too) through the native kv host.
mod kv_trace_parity {
    use orbital_core::os::{OsError, OsOp};
    use orbital_core::runtime::kv_host::{json_text, KvStores};
    use orbital_core::runtime::os_host::{answer_os, KvMode, OsGrants, OsHandles, OsHost};
    use orbital_core::runtime::resumable::{CallSite, IoAnswer, IoRequest};
    use orbital_core::Value;
    use serde_json::Value as Json;
    use std::collections::BTreeMap;
    use std::path::Path;

    fn v(json: Json) -> Value {
        serde_json::from_value(json).expect("value")
    }

    fn replay(kv: KvStores, steps: &[Json]) {
        let handles = OsHandles { kv, ..OsHandles::default() };
        let grants = OsGrants { kv: BTreeMap::from([("rows".to_string(), vec![KvMode::Read, KvMode::Write])]), ..OsGrants::default() };
        let on_event = |_: Option<&CallSite>, _: Value| {};
        let host = OsHost { grants: &grants, handles: &handles, on_event: &on_event };
        for (i, step) in steps.iter().enumerate() {
            let name = step["op"].as_str().unwrap();
            let op = OsOp::from_name(name).unwrap_or_else(|| panic!("{name} is not an op"));
            let args: Vec<Value> = step["args"].as_array().unwrap().iter().cloned().map(v).collect();
            let answered: Result<Value, OsError> = match answer_os(Some(&host), &IoRequest::Os { op, args, key: None, site: None }) {
                IoAnswer::Os(answer) => answer,
                other => panic!("step {i} {name}: {other:?}"),
            };
            match (answered, step.get("expectError").and_then(Json::as_str)) {
                (Ok(value), None) => assert_eq!(json_text(&value), json_text(&v(step["expect"].clone())), "step {i} {name}"),
                (Err(error), Some(kind)) => assert_eq!(error.kind.as_str(), kind, "step {i} {name}: {}", error.message),
                (other, want) => panic!("step {i} {name}: answered {other:?}, expected {want:?} / {}", step["expect"]),
            }
        }
    }

    #[test]
    fn the_shared_kv_trace_replays_byte_identical_on_memory_and_directory_stores() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../orbital-shell-typescript/renderers/web/test/fixtures/kv-trace.json");
        let trace: Json = serde_json::from_str(&std::fs::read_to_string(&path).expect("the shared kv trace")).unwrap();
        let (app, locale) = (trace["app"].as_str().unwrap(), trace["locale"].as_str().unwrap());
        let steps = trace["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 20);
        replay(KvStores::memory(app, locale), steps);
        let dir = std::env::temp_dir().join(format!("kv-trace-parity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        replay(KvStores::directory(&dir, app, locale), steps);
        let bytes = std::fs::read_to_string(dir.join("kv").join(app).join(locale).join("rows.json")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(bytes, r#"{"order-line/1":false,"order.log/0001":{"op":"create"},"order/3":"three"}"#);
    }
}
