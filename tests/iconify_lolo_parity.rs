// H.6 parity: every recorded iconify fixture, replayed through the kernel against `std-iconify.lolo`, makes the
// request the manual provider made (method, URL, compared headers, body, timeout) and emits the recorded outcome.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orbital_core::effects::Effect;
use orbital_core::runtime::http_service::{HttpRequest, HttpResponse, TransportError};
use orbital_core::runtime::resumable::{drive_blocking, BlockingHost, IoAnswer, IoRequest};
use orbital_core::runtime::{EntitySource, InMemoryEntityCache, KernelConfig, RuntimeKernel};
use orbital_core::{OrbitalSchema, RuntimeEnvironment, Value};
use serde_json::{json, Value as Json};

const TRAIT: &str = "IconifyClient";
const DEFAULT_TIMEOUT_MS: u64 = 6000;
const COMPARED_HEADERS: [&str; 8] = [
    "content-type",
    "accept",
    "authorization",
    "x-api-key",
    "anthropic-version",
    "anthropic-beta",
    "openai-organization",
    "user-agent",
];
const SECRET_HEADERS: [&str; 2] = ["authorization", "x-api-key"];
const ENV_KNOBS: [(&str, &str); 2] = [("ICONIFY_USER_AGENT", "userAgent"), ("ICONIFY_TIMEOUT_MS", "timeoutMs")];

fn atom_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../packages/almadar-std/behaviors/lolo/infra/atoms/std-iconify.lolo")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../orbital-integrations/fixtures/iconify")
}

fn fixtures() -> Vec<Json> {
    let dir = fixtures_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} is missing ({e})", dir.display()))
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|path| {
            let text = std::fs::read_to_string(path).expect("read fixture");
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        })
        .collect()
}

fn text<'a>(value: &'a Json, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_else(|| panic!("fixture field {key} is not text: {value}"))
}

fn is_ok(fixture: &Json) -> bool {
    fixture["outcome"]["ok"].as_bool().expect("outcome.ok")
}

fn events_of(action: &str) -> (&'static str, &'static str) {
    match action {
        "svgExists" => ("ICONIFY_SVG_EXISTS", "ICONIFY_SVG_EXISTS_RESULT"),
        "search" => ("ICONIFY_SEARCH", "ICONIFY_SEARCH_RESULT"),
        "getIconBody" => ("ICONIFY_GET_ICON_BODY", "ICONIFY_ICON_BODY_RESULT"),
        other => panic!("no atom event for action {other}"),
    }
}

struct Replay {
    case: String,
    script: Mutex<VecDeque<Json>>,
    sent: Mutex<Vec<HttpRequest>>,
}

impl Replay {
    fn new(fixture: &Json) -> Arc<Self> {
        Arc::new(Self {
            case: text(fixture, "case").to_string(),
            script: Mutex::new(fixture["exchanges"].as_array().expect("exchanges").iter().cloned().collect()),
            sent: Mutex::new(Vec::new()),
        })
    }

    fn unused(&self) -> usize {
        self.script.lock().expect("script").len()
    }

    fn semantic(headers: &[(String, String)]) -> BTreeMap<String, String> {
        headers
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.clone()))
            .filter(|(k, _)| COMPARED_HEADERS.contains(&k.as_str()))
            .map(|(k, v)| if SECRET_HEADERS.contains(&k.as_str()) { (k, "<redacted>".to_string()) } else { (k, v) })
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
        assert_eq!(Self::semantic(&request.headers), expected, "{case}: headers");
        assert_eq!(request.text(), text(recorded, "body"), "{case}: request body");
        let reply = &exchange["response"];
        Ok(HttpResponse {
            status: reply["status"].as_u64().expect("status") as u16,
            headers: Vec::new(),
            body: text(reply, "body").as_bytes().to_vec(),
        })
    }
}

fn expected_timeout(fixture: &Json) -> Duration {
    let ms = fixture["env"]["ICONIFY_TIMEOUT_MS"]
        .as_str()
        .map_or(DEFAULT_TIMEOUT_MS, |t| t.parse().expect("ICONIFY_TIMEOUT_MS is a number"));
    Duration::from_millis(ms)
}

fn lowered(fixture: &Json) -> OrbitalSchema {
    let path = atom_path();
    let schema = orbital_test_support::lower_lolo_file(&path)
        .unwrap_or_else(|e| panic!("lower {}: {e}", path.display()));
    let mut json = serde_json::to_value(&schema).expect("schema json");
    for (variable, knob) in ENV_KNOBS {
        let Some(raw) = fixture["env"][variable].as_str() else { continue };
        let value = if knob == "timeoutMs" { json!(raw.parse::<u64>().expect("number")) } else { json!(raw) };
        let mut patched = false;
        for orbital in json["orbitals"].as_array_mut().expect("orbitals") {
            for t in orbital["traits"].as_array_mut().into_iter().flatten() {
                if let Some(entry) = t["config"].get_mut(knob) {
                    entry["default"] = value.clone();
                    patched = true;
                }
            }
        }
        assert!(patched, "{}: the atom declares no config knob {knob}", text(fixture, "case"));
    }
    serde_json::from_value(json).expect("schema back from json")
}

struct Run {
    emitted: Vec<Json>,
    sent: Vec<HttpRequest>,
    unused: usize,
}

fn run(fixture: &Json) -> Run {
    let (input, result) = events_of(text(fixture, "action"));
    let store: Arc<dyn EntitySource> = Arc::new(InMemoryEntityCache::new());
    let kernel = RuntimeKernel::new(RuntimeEnvironment::Server, store.clone(), KernelConfig::default());
    kernel.register_schema(lowered(fixture)).expect("register std-iconify");
    let replay = Replay::new(fixture);
    let payload: Value = serde_json::from_value(fixture["params"].clone()).expect("params as a payload");
    let progress = kernel
        .start_dispatch(TRAIT, None, input, payload, None, None)
        .unwrap_or_else(|e| panic!("{}: dispatch: {e:?}", replay.case));
    let answer = |request: &IoRequest| match request {
        IoRequest::Http { request, .. } => IoAnswer::Http(replay.answer(request)),
        other => BlockingHost { source: store.as_ref(), service: None, substrate: None }.answer(other),
    };
    let outcomes = drive_blocking(&kernel, progress, &answer)
        .unwrap_or_else(|e| panic!("{}: drive: {e:?}", replay.case));
    let mut emitted = Vec::new();
    for (_, outcome) in &outcomes {
        assert_eq!(outcome.failure, None, "{}: an effect failed", replay.case);
        for effect in &outcome.effect_results {
            if let Effect::Emit { event, payload } = &effect.effect {
                if event == result {
                    emitted.push(serde_json::to_value(payload).expect("payload json"));
                }
            }
        }
    }
    let sent = replay.sent.lock().expect("sent").clone();
    Run { emitted, sent, unused: replay.unused() }
}

fn check(fixture: &Json) {
    let case = text(fixture, "case");
    let run = run(fixture);
    assert_eq!(run.unused, 0, "{case}: recorded requests the atom never sent");
    assert_eq!(run.sent.len(), fixture["exchanges"].as_array().expect("exchanges").len(), "{case}: request count");
    for request in &run.sent {
        assert_eq!(request.timeout, Some(expected_timeout(fixture)), "{case}: timeout");
    }
    assert_eq!(run.emitted, vec![fixture["outcome"]["data"].clone()], "{case}: emitted result");
}

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = fixtures();
    assert!(all.len() >= 22, "expected the recorded iconify cases, found {}", all.len());
    let replayed: Vec<&Json> = all.iter().filter(|f| is_ok(f)).collect();
    assert_eq!(replayed.len(), 18, "the cases with a recorded result");
    for fixture in replayed {
        check(fixture);
    }
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    let rejected: Vec<Json> = fixtures().into_iter().filter(|f| !is_ok(f)).collect();
    assert_eq!(rejected.len(), 4);
    for fixture in &rejected {
        assert_eq!(fixture["outcome"]["code"], "VALIDATION_ERROR", "{}", text(fixture, "case"));
        assert!(fixture["exchanges"].as_array().expect("exchanges").is_empty(), "{}", text(fixture, "case"));
    }
}

fn fixture_named(case: &str) -> Json {
    fixtures().into_iter().find(|f| text(f, "case") == case).unwrap_or_else(|| panic!("fixture {case}"))
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = fixture_named("search");
    let url = text(&fixture["exchanges"][0]["request"], "url").replace("limit=3", "limit=4");
    fixture["exchanges"][0]["request"]["url"] = json!(url);
    check(&fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = fixture_named("search");
    fixture["outcome"]["data"]["icons"] = json!(["logos:github-icon"]);
    check(&fixture);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    let known: Vec<&str> = ENV_KNOBS.iter().map(|(variable, _)| *variable).collect();
    for fixture in fixtures() {
        let case = text(&fixture, "case");
        for name in fixture["env"].as_object().expect("env").keys() {
            assert!(known.contains(&name.as_str()), "{case}: unexpected env {name}");
        }
        for exchange in fixture["exchanges"].as_array().expect("exchanges") {
            for header in SECRET_HEADERS {
                assert!(exchange["request"]["headers"].get(header).is_none(), "{case}: {header}");
            }
        }
    }
    let source = std::fs::read_to_string(atom_path()).expect("read std-iconify.lolo");
    assert!(!source.contains(": secret"), "the keyless atom declares a secret knob");
}
