#![allow(dead_code)]
// H.6 parity harness: replays every recorded fixture of a service through the kernel against its std `.lolo`
// atom and compares the request the atom makes and the event it emits with the recorded exchange and outcome.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orbital_core::effects::Effect;
use orbital_core::runtime::http_service::{HttpRequest, HttpResponse, TransportError};
use orbital_core::runtime::resumable::{drive_blocking, BlockingHost, IoAnswer, IoRequest};
use orbital_core::runtime::{EntitySource, InMemoryEntityCache, DispatchClock, KernelConfig, RuntimeKernel};
use orbital_core::{OrbitalSchema, RuntimeEnvironment, Value};
use serde_json::{json, Value as Json};

pub const REDACTED: &str = "<redacted>";
pub const GENERATED: &str = "<generated>";
pub const SECRET_HEADERS: [&str; 2] = ["authorization", "x-api-key"];

pub struct ActionEvents {
    pub action: &'static str,
    pub input: &'static str,
    pub result: &'static str,
    pub failure: Option<&'static str>,
}

pub struct ParitySpec {
    /// Path of the atom under `packages/`, e.g. `almadar-std/behaviors/lolo/infra/atoms/std-iconify.lolo`.
    pub atom: &'static str,
    pub trait_name: &'static str,
    /// The service directory under `orbital-integrations/fixtures/`.
    pub fixtures_dir: &'static str,
    pub events: &'static [ActionEvents],
    /// `(ENV_VAR, knob)`: the fixture env value patches the knob default; a `timeoutMs` knob is a number.
    pub env_knobs: &'static [(&'static str, &'static str)],
    /// Env vars answered as `IoRequest::Secret` from the fixture env.
    pub secret_env: &'static [&'static str],
    pub compared_headers: &'static [&'static str],
    pub default_timeout_ms: u64,
    /// `(knob, origin)`: the knob is set to the test origin and `{base}` in a fixture is replaced by it.
    pub base: Option<(&'static str, &'static str)>,
}

pub struct Run {
    pub results: Vec<Json>,
    pub failures: Vec<Json>,
    pub sent: Vec<HttpRequest>,
    pub unused: usize,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn atom_path(spec: &ParitySpec) -> PathBuf {
    root().join("../../../packages").join(spec.atom)
}

pub fn text<'a>(value: &'a Json, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_else(|| panic!("fixture field {key} is not text: {value}"))
}

pub fn is_ok(fixture: &Json) -> bool {
    fixture["outcome"]["ok"].as_bool().expect("outcome.ok")
}

pub fn fixtures(spec: &ParitySpec) -> Vec<Json> {
    let dir = root().join("../orbital-integrations/fixtures").join(spec.fixtures_dir);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} is missing ({e})", dir.display()))
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|path| {
            let mut text = std::fs::read_to_string(path).expect("read fixture");
            if let Some((_, origin)) = spec.base {
                text = text.replace("{base}", origin);
            }
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        })
        .collect()
}

pub fn fixture_named(spec: &ParitySpec, case: &str) -> Json {
    fixtures(spec).into_iter().find(|f| text(f, "case") == case).unwrap_or_else(|| panic!("fixture {case}"))
}

fn events_of<'a>(spec: &'a ParitySpec, action: &str) -> &'a ActionEvents {
    spec.events
        .iter()
        .find(|e| e.action == action)
        .unwrap_or_else(|| panic!("no atom event for action {action}"))
}

struct Replay<'a> {
    spec: &'a ParitySpec,
    case: String,
    script: Mutex<VecDeque<Json>>,
    sent: Mutex<Vec<HttpRequest>>,
}

impl<'a> Replay<'a> {
    fn new(spec: &'a ParitySpec, fixture: &Json) -> Arc<Self> {
        Arc::new(Self {
            spec,
            case: text(fixture, "case").to_string(),
            script: Mutex::new(fixture["exchanges"].as_array().expect("exchanges").iter().cloned().collect()),
            sent: Mutex::new(Vec::new()),
        })
    }

    fn unused(&self) -> usize {
        self.script.lock().expect("script").len()
    }

    fn semantic(&self, headers: &[(String, String)], recorded: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        headers
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.clone()))
            .filter(|(k, _)| self.spec.compared_headers.contains(&k.as_str()))
            .map(|(k, v)| {
                let value = if SECRET_HEADERS.contains(&k.as_str()) {
                    REDACTED.to_string()
                } else if recorded.get(&k).is_some_and(|r| r == GENERATED) {
                    GENERATED.to_string()
                } else {
                    v
                };
                (k, value)
            })
            .collect()
    }

    fn answer(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let case = &self.case;
        self.sent.lock().expect("sent").push(request.clone());
        let exchange = self.script.lock().expect("script").pop_front().unwrap_or_else(|| {
            panic!("{case}: the atom sent an unrecorded request: {} {}", request.method, request.url)
        });
        let recorded = &exchange["request"];
        assert_eq!(request.method, text(recorded, "method"), "{case}: method");
        assert_eq!(request.url, text(recorded, "url"), "{case}: url");
        let expected: BTreeMap<String, String> = recorded["headers"]
            .as_object()
            .expect("recorded headers")
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().expect("header text").to_string()))
            .collect();
        assert_eq!(self.semantic(&request.headers, &expected), expected, "{case}: headers");
        assert_eq!(request.text(), text(recorded, "body"), "{case}: request body");
        let reply = &exchange["response"];
        let headers = reply["headers"]
            .as_object()
            .map(|h| h.iter().map(|(k, v)| (k.clone(), v.as_str().expect("header text").to_string())).collect())
            .unwrap_or_default();
        Ok(HttpResponse {
            status: reply["status"].as_u64().expect("status") as u16,
            headers,
            body: text(reply, "body").as_bytes().to_vec(),
        })
    }
}

fn expected_timeout(spec: &ParitySpec, fixture: &Json) -> Duration {
    let ms = spec
        .env_knobs
        .iter()
        .find(|(_, knob)| *knob == "timeoutMs")
        .and_then(|(variable, _)| fixture["env"][*variable].as_str())
        .map_or(spec.default_timeout_ms, |t| t.parse().expect("timeout env is a number"));
    Duration::from_millis(ms)
}

fn lowered(spec: &ParitySpec, fixture: &Json) -> OrbitalSchema {
    let path = atom_path(spec);
    let schema = orbital_test_support::lower_lolo_file(&path)
        .unwrap_or_else(|e| panic!("lower {}: {e}", path.display()));
    let mut json = serde_json::to_value(&schema).expect("schema json");
    for (variable, knob) in spec.env_knobs {
        let Some(raw) = fixture["env"][*variable].as_str() else { continue };
        let value = if *knob == "timeoutMs" { json!(raw.parse::<u64>().expect("number")) } else { json!(raw) };
        let mut patched = false;
        for orbital in json["orbitals"].as_array_mut().expect("orbitals") {
            for t in orbital["traits"].as_array_mut().into_iter().flatten() {
                if let Some(entry) = t["config"].get_mut(*knob) {
                    entry["default"] = value.clone();
                    patched = true;
                }
            }
        }
        assert!(patched, "{}: the atom declares no config knob {knob}", text(fixture, "case"));
    }
    if let Some((knob, origin)) = spec.base {
        let mut patched = false;
        for orbital in json["orbitals"].as_array_mut().expect("orbitals") {
            for t in orbital["traits"].as_array_mut().into_iter().flatten() {
                if let Some(entry) = t["config"].get_mut(knob) {
                    entry["default"] = json!(origin);
                    patched = true;
                }
            }
        }
        assert!(patched, "{}: the atom declares no base knob {knob}", text(fixture, "case"));
    }
    serde_json::from_value(json).expect("schema back from json")
}

fn pinned_clock(fixture: &Json) -> Option<DispatchClock> {
    let iso = fixture.get("now")?.as_str().unwrap_or_else(|| panic!("{}: `now` must be an ISO time string", text(fixture, "case")));
    let at = chrono::DateTime::parse_from_rfc3339(iso)
        .unwrap_or_else(|e| panic!("{}: `now` {iso}: {e}", text(fixture, "case")))
        .timestamp_millis() as f64;
    Some(Arc::new(move || at))
}

pub fn run(spec: &ParitySpec, fixture: &Json) -> Run {
    let events = events_of(spec, text(fixture, "action"));
    let store: Arc<dyn EntitySource> = Arc::new(InMemoryEntityCache::new());
    let kernel = RuntimeKernel::new(RuntimeEnvironment::Server, store.clone(), KernelConfig { clock: pinned_clock(fixture), ..KernelConfig::default() });
    kernel.register_schema(lowered(spec, fixture)).unwrap_or_else(|e| panic!("register {}: {e:?}", spec.atom));
    let replay = Replay::new(spec, fixture);
    let payload: Value = serde_json::from_value(fixture["params"].clone()).expect("params as a payload");
    let progress = kernel
        .start_dispatch(spec.trait_name, None, events.input, payload, None, None)
        .unwrap_or_else(|e| panic!("{}: dispatch: {e:?}", replay.case));
    let answer = |request: &IoRequest| match request {
        IoRequest::Http { request, .. } => IoAnswer::Http(replay.answer(request)),
        IoRequest::Secret { name, .. } => {
            assert!(spec.secret_env.contains(&name.as_str()), "{}: the atom asked for undeclared secret {name}", replay.case);
            IoAnswer::Secret(Ok(fixture["env"][name.as_str()].as_str().map(str::to_string)))
        }
        IoRequest::SecretSet { .. } => IoAnswer::SecretSet(Ok(())),
        other => BlockingHost { source: store.as_ref(), service: None, substrate: None }.answer(other),
    };
    let outcomes = drive_blocking(&kernel, progress, &answer)
        .unwrap_or_else(|e| panic!("{}: drive: {e:?}", replay.case));
    let mut results = Vec::new();
    let mut failures = Vec::new();
    for (_, outcome) in &outcomes {
        assert_eq!(outcome.failure, None, "{}: an effect failed", replay.case);
        for effect in &outcome.effect_results {
            if let Effect::Emit { event, payload } = &effect.effect {
                let value = by_value(serde_json::to_value(payload).expect("payload json"));
                if event == events.result {
                    results.push(value);
                } else if events.failure.is_some_and(|f| f == event) {
                    failures.push(value);
                }
            }
        }
    }
    let sent = replay.sent.lock().expect("sent").clone();
    Run { results, failures, sent, unused: replay.unused() }
}

/// The kernel's numbers are f64, so `98765` serializes as `98765.0`; JSON readers see one number, and so does this.
fn by_value(value: Json) -> Json {
    match value {
        Json::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 9.007_199_254_740_992e15 => json!(f as i64),
            _ => Json::Number(n),
        },
        Json::Array(items) => Json::Array(items.into_iter().map(by_value).collect()),
        Json::Object(map) => Json::Object(map.into_iter().map(|(k, v)| (k, by_value(v))).collect()),
        other => other,
    }
}

pub fn check(spec: &ParitySpec, fixture: &Json) {
    let case = text(fixture, "case");
    let run = run(spec, fixture);
    assert_eq!(run.unused, 0, "{case}: recorded requests the atom never sent");
    assert_eq!(run.sent.len(), fixture["exchanges"].as_array().expect("exchanges").len(), "{case}: request count");
    for request in &run.sent {
        assert_eq!(request.timeout, Some(expected_timeout(spec, fixture)), "{case}: timeout");
    }
    if is_ok(fixture) {
        assert!(run.failures.is_empty(), "{case}: unexpected failure event {:?}", run.failures);
        assert_eq!(run.results, vec![by_value(fixture["outcome"]["data"].clone())], "{case}: emitted result");
    } else {
        let outcome = &fixture["outcome"];
        let expected = json!({"code": outcome["code"], "message": outcome["message"]});
        assert!(run.results.is_empty(), "{case}: unexpected result event {:?}", run.results);
        assert_eq!(run.failures, vec![expected], "{case}: emitted failure");
    }
}

/// D-A80: the listed cases are contract-validation errors that send nothing; they are the only unreplayed ones.
pub fn assert_validation_only_excluded(spec: &ParitySpec, cases: &[&str]) {
    let rejected: Vec<String> = fixtures(spec)
        .iter()
        .filter(|f| !is_ok(f) && f["outcome"]["code"] == "VALIDATION_ERROR")
        .map(|f| text(f, "case").to_string())
        .collect();
    let mut listed: Vec<String> = cases.iter().map(|c| c.to_string()).collect();
    listed.sort();
    let mut found = rejected.clone();
    found.sort();
    assert_eq!(found, listed, "the validation-only cases");
    for fixture in fixtures(spec).iter().filter(|f| cases.contains(&text(f, "case"))) {
        assert!(fixture["exchanges"].as_array().expect("exchanges").is_empty(), "{}", text(fixture, "case"));
    }
}

/// Every fixture that is not excluded replays; returns how many were replayed.
pub fn replay_all_except(spec: &ParitySpec, excluded: &[&str]) -> usize {
    let mut replayed = 0;
    for fixture in fixtures(spec) {
        if excluded.contains(&text(&fixture, "case")) {
            continue;
        }
        check(spec, &fixture);
        replayed += 1;
    }
    replayed
}

/// Control: a fixture whose first recorded URL had `from` replaced by `to` must fail the replay on the url.
pub fn drifted_url_control(spec: &ParitySpec, case: &str, from: &str, to: &str) {
    let mut fixture = fixture_named(spec, case);
    let url = text(&fixture["exchanges"][0]["request"], "url").replace(from, to);
    assert_ne!(url, text(&fixture["exchanges"][0]["request"], "url"), "the control changed nothing");
    fixture["exchanges"][0]["request"]["url"] = json!(url);
    check(spec, &fixture);
}

/// Control: a fixture whose recorded outcome data at `key` is replaced must fail the replay on the emitted result.
pub fn drifted_outcome_control(spec: &ParitySpec, case: &str, key: &str, value: Json) {
    let mut fixture = fixture_named(spec, case);
    assert_ne!(fixture["outcome"]["data"][key], value, "the control changed nothing");
    fixture["outcome"]["data"][key] = value;
    check(spec, &fixture);
}

/// Fixtures hold only test values: secret-named env (`*_KEY`, `*_TOKEN`, `*_SECRET`) is `test-` prefixed or empty, secret headers are redacted.
pub fn assert_no_secrets(spec: &ParitySpec) {
    let all = fixtures(spec);
    for fixture in &all {
        let case = text(fixture, "case");
        for (name, value) in fixture["env"].as_object().expect("env") {
            let known = spec.env_knobs.iter().any(|(v, _)| v == name) || spec.secret_env.contains(&name.as_str());
            let secretish = ["_KEY", "_TOKEN", "_SECRET", "_SECRET_ID"].iter().any(|s| name.ends_with(s));
            assert!(known || !secretish, "{case}: unexpected secret env {name}");
            // A credential slot may carry an identifier (an account SID as the basic-auth user); only
            // secret-named vars must hold test values.
            if secretish {
                let v = value.as_str().expect("env text");
                assert!(v.starts_with("test-") || v.is_empty(), "{case}: {name}");
            }
        }
        for exchange in fixture["exchanges"].as_array().expect("exchanges") {
            for header in SECRET_HEADERS {
                if let Some(value) = exchange["request"]["headers"].get(header) {
                    assert_eq!(value, REDACTED, "{case}: {header}");
                }
            }
        }
    }
    let source = std::fs::read_to_string(atom_path(spec)).expect("read the atom");
    assert_eq!(
        source.contains(": secret"),
        !spec.secret_env.is_empty(),
        "the atom's secret knobs must match the spec's secret_env"
    );
}
