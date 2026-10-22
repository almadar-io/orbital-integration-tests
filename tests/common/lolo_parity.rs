#![allow(dead_code)]
// H.6 parity harness: replays every recorded fixture of a service through the kernel against its std `.lolo`
// atom and compares the request the atom makes and the event it emits with the recorded exchange and outcome.
//
// The atom is lowered (L1), its knobs are patched from the fixture, and the result is resolved through the compiler,
// so an atom that `uses` another (std-webhook composes std-retry) replays as the composed program.
//
// Fixture shape: `{ case, action, env, params, exchanges, osExchanges?, outcome, now? }`.
// - `exchanges`: `{ request: { method, url, headers, body }, response: { status, headers?, body } }`, matched in order.
// - `osExchanges`: the `os/*` asks the atom makes, matched in order among themselves:
//   `{ "os": { "op": "os/tcp-send", "args": [..] }, "answer": { "ok": <value> } }` or
//   `{ "os": {..}, "answer": { "error": { "kind": "io", "message": ".." } } }`. `args` are the op's positional arguments
//   and compare like recorded text (tokens such as `<hex8>` match a generated value); an unrecorded, extra or out-of-order
//   ask fails the replay.
// - `outcome`: `{ ok, data }` or `{ ok: false, code, message }`; a spec's `failure_extras` adds fields the failure event
//   carries beyond `code` and `message`.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orbital_core::effects::Effect;
use orbital_core::ir::PageFactField;
use orbital_core::oauth::ask::{OAuthAsk, OAuthReply};
use orbital_core::oauth::store::MemoryOAuthStore;
use orbital_core::runtime::http_request::{BeaconOutcome, CredentialSlot, SecretScope};
use orbital_core::runtime::http_service::{
    credential_status_connected, HttpRequest, HttpResponse, SecretSource, TransportError,
};
use orbital_core::runtime::os::{OsError, OsErrorKind, OsOp};
use orbital_core::runtime::resumable::{
    drive_blocking, BlockingHost, IoAnswer, IoRequest, PageFacts,
};
use orbital_core::runtime::{
    DispatchClock, EntitySource, InMemoryEntityCache, KernelConfig, RuntimeKernel,
};
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
    /// The provider's result is not a record: the atom carries it under this result field.
    pub data_field: Option<&'static str>,
}

pub struct ParitySpec {
    /// Path of the atom under `packages/`, e.g. `almadar-std/behaviors/lolo/connect/atoms/std-iconify.lolo`.
    pub atom: &'static str,
    pub trait_name: &'static str,
    /// The service directory under `fixtures/` here.
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
    /// `{base}` in a fixture is replaced by this origin without patching a knob (the atom declares it in egress).
    pub fixture_origin: Option<&'static str>,
    /// `(ENV_VAR, knob)`: the knob is `true` when the fixture env value is present and not empty, else `false`.
    pub presence_knobs: &'static [(&'static str, &'static str)],
    /// `(case, knob, value)`: the knob is set for that fixture case only.
    pub case_knobs: &'static [(&'static str, &'static str, KnobValue)],
    /// What a recorded `env_knobs` value that does not read as the knob's type does.
    pub unreadable_knob: UnreadableKnob,
    /// Fields the failure event carries beyond `code` and `message`, and the value each takes for a fixture.
    pub failure_extras: fn(&Json) -> Vec<(&'static str, Json)>,
    /// The failure event field that carries the outcome's `message` (`error` on atoms that follow the `{error, code}` shape).
    pub failure_message_field: &'static str,
}

pub fn no_failure_extras(_: &Json) -> Vec<(&'static str, Json)> {
    Vec::new()
}

impl ParitySpec {
    pub const DEFAULT: ParitySpec = ParitySpec {
        atom: "",
        trait_name: "",
        fixtures_dir: "",
        events: &[],
        env_knobs: &[],
        secret_env: &[],
        compared_headers: &[],
        default_timeout_ms: 0,
        base: None,
        fixture_origin: None,
        presence_knobs: &[],
        case_knobs: &[],
        unreadable_knob: UnreadableKnob::Fail,
        failure_extras: no_failure_extras,
        failure_message_field: "message",
    };
}

#[derive(Clone, Copy)]
pub enum KnobValue {
    Number(u64),
    Bool(bool),
    Text(&'static str),
}

impl KnobValue {
    fn json(self) -> Json {
        match self {
            Self::Number(n) => json!(n),
            Self::Bool(b) => json!(b),
            Self::Text(t) => json!(t),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UnreadableKnob {
    /// The recorded value must read as the knob's type; otherwise the replay fails with [`KnobError::Unreadable`].
    Fail,
    /// The fixture records that an unreadable value leaves the atom's own default in force.
    UseDefault,
}

#[derive(Debug, PartialEq, Eq)]
pub enum KnobError {
    Unreadable {
        case: String,
        variable: String,
        knob: String,
        raw: String,
    },
    Undeclared {
        case: String,
        knob: String,
    },
}

impl std::fmt::Display for KnobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable {
                case,
                variable,
                knob,
                raw,
            } => {
                write!(
                    f,
                    "{case}: {variable}={raw:?} is not a number for knob {knob}"
                )
            }
            Self::Undeclared { case, knob } => {
                write!(f, "{case}: the atom declares no config knob {knob}")
            }
        }
    }
}

/// One `os/*` ask the atom made.
#[derive(Debug, Clone)]
pub struct OsCall {
    pub op: OsOp,
    pub args: Vec<Json>,
}

pub struct Run {
    pub results: Vec<Json>,
    pub failures: Vec<Json>,
    pub sent: Vec<HttpRequest>,
    pub os_sent: Vec<OsCall>,
    pub unused_os: usize,
    pub unused: usize,
    pub unused_entropy: usize,
    /// The virtual time (ms) each request was sent at: the clock when it was asked plus its `delayMs`, in order.
    pub send_times: Vec<u64>,
}

pub struct FixtureCredential {
    pub slot: CredentialSlot,
    pub principal: Option<String>,
    pub value: Option<String>,
    pub ttl_ms: Option<u64>,
}

#[derive(Default)]
pub struct FixtureAuthority {
    pub credentials: Vec<FixtureCredential>,
    pub now_ms: Option<u64>,
}

#[derive(Clone, Default)]
pub enum RecordedBody {
    #[default]
    RecordedText,
    Bytes(Vec<u8>),
}

impl RecordedBody {
    fn bytes(&self, recorded: &Json) -> Vec<u8> {
        match self {
            Self::RecordedText => text(recorded, "body").as_bytes().to_vec(),
            Self::Bytes(bytes) => bytes.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub enum Deadline {
    #[default]
    Default,
    Exact(Option<Duration>),
}

#[derive(Clone, Default)]
pub struct ExchangeContract {
    pub request_body: RecordedBody,
    pub response_body: RecordedBody,
    pub timeout: Deadline,
}

#[derive(Default)]
pub struct ReplayStep {
    pub user: Value,
    pub at_ms: Option<u64>,
    pub entropy: Vec<Vec<u8>>,
    pub exchanges: Vec<ExchangeContract>,
}

struct CredentialValue {
    value: String,
    expires_ms: Option<f64>,
}

impl CredentialValue {
    fn new(value: Option<String>, ttl_ms: Option<u64>, now: f64) -> Option<Self> {
        value.map(|value| Self {
            value,
            expires_ms: ttl_ms.map(|ttl| now + ttl as f64),
        })
    }

    fn read(&self, now: f64) -> Option<String> {
        self.expires_ms
            .is_none_or(|expires| now < expires)
            .then(|| self.value.clone())
    }
}

#[derive(Default)]
struct CredentialState {
    initial: Option<CredentialValue>,
    minted: Option<CredentialValue>,
}

struct FixtureHost {
    clock: Arc<Mutex<Option<f64>>>,
    credentials: Mutex<BTreeMap<(String, Option<String>), CredentialState>>,
    /// Every durable rotation the atom asked for, in order: `(credential, principal)`.
    rotations: Mutex<Vec<(String, Option<String>)>>,
    /// The OAuth state an `oauth-client` flow keeps (pending grants, connections, metadata), on the fixture clock.
    oauth: MemoryOAuthStore,
}

impl FixtureHost {
    fn new(spec: &ParitySpec, fixture: &Json, authority: FixtureAuthority) -> Arc<Self> {
        let clock = authority
            .now_ms
            .map(|now| now as f64)
            .or_else(|| fixture_now(fixture));
        let clock = Arc::new(Mutex::new(clock));
        let oauth_clock = clock.clone();
        let host = Arc::new(Self {
            clock,
            credentials: Mutex::new(BTreeMap::new()),
            rotations: Mutex::new(Vec::new()),
            oauth: MemoryOAuthStore::new(move || {
                oauth_clock
                    .lock()
                    .expect("fixture clock")
                    .unwrap_or_else(orbital_core::time_util::now_millis_f64) as i64
            }),
        });
        let now = host.now();
        let mut credentials = host.credentials.lock().expect("fixture credentials");
        for name in spec.secret_env {
            let value = fixture["env"]
                .get(*name)
                .map(|value| {
                    value
                        .as_str()
                        .unwrap_or_else(|| panic!("fixture credential {name} must be text"))
                        .to_string()
                })
                .filter(|value| !value.is_empty());
            credentials.insert(
                ((*name).to_string(), None),
                CredentialState {
                    initial: CredentialValue::new(value, None, now),
                    minted: None,
                },
            );
        }
        for credential in authority.credentials {
            assert_eq!(
                credential.slot.scope == SecretScope::User,
                credential.principal.is_some(),
                "fixture credential scope/principal"
            );
            credentials.insert(
                (credential.slot.name, credential.principal),
                CredentialState {
                    initial: CredentialValue::new(credential.value, credential.ttl_ms, now),
                    minted: None,
                },
            );
        }
        drop(credentials);
        host
    }

    fn now(&self) -> f64 {
        self.clock
            .lock()
            .expect("fixture clock")
            .unwrap_or_else(orbital_core::time_util::now_millis_f64)
    }

    fn set_time(&self, at_ms: Option<u64>) {
        if let Some(at) = at_ms {
            *self.clock.lock().expect("fixture clock") = Some(at as f64);
        }
    }

    fn with_credential<T>(
        &self,
        name: &str,
        principal: Option<&str>,
        f: impl FnOnce(&mut CredentialState) -> T,
    ) -> T {
        let mut credentials = self.credentials.lock().expect("fixture credentials");
        let entry = credentials
            .get_mut(&(name.to_string(), principal.map(str::to_string)))
            .unwrap_or_else(|| panic!("undeclared credential {name} for principal {principal:?}"));
        f(entry)
    }
}

impl SecretSource for FixtureHost {
    fn secret(&self, name: &str) -> Result<Option<String>, String> {
        self.secret_of(name, None)
    }

    fn secret_of(&self, name: &str, principal: Option<&str>) -> Result<Option<String>, String> {
        let now = self.now();
        Ok(self.with_credential(name, principal, |entry| {
            if let Some(value) = entry.minted.as_ref().and_then(|value| value.read(now)) {
                return Some(value);
            }
            entry.minted = None;
            entry.initial.as_ref().and_then(|value| value.read(now))
        }))
    }

    fn store_secret(&self, name: &str, value: &str, ttl_ms: u64) -> Result<(), String> {
        self.store_secret_of(name, value, ttl_ms, None)
    }

    fn store_secret_of(
        &self,
        name: &str,
        value: &str,
        ttl_ms: u64,
        principal: Option<&str>,
    ) -> Result<(), String> {
        let now = self.now();
        self.with_credential(name, principal, |entry| {
            entry.minted = CredentialValue::new(Some(value.to_string()), Some(ttl_ms), now);
        });
        Ok(())
    }

    fn oauth_of(&self, ask: &OAuthAsk, principal: Option<&str>) -> Result<OAuthReply, String> {
        self.oauth.answer(ask, principal)
    }

    fn rotate_secret_of(
        &self,
        name: &str,
        value: &str,
        principal: Option<&str>,
    ) -> Result<(), String> {
        let now = self.now();
        self.with_credential(name, principal, |entry| {
            entry.initial = CredentialValue::new(Some(value.to_string()), None, now);
        });
        self.rotations
            .lock()
            .expect("fixture rotations")
            .push((name.to_string(), principal.map(str::to_string)));
        Ok(())
    }

    fn forget_of(&self, name: &str, principal: Option<&str>) -> Result<(), String> {
        self.with_credential(name, principal, |entry| *entry = CredentialState::default());
        Ok(())
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn atom_path(spec: &ParitySpec) -> PathBuf {
    root().join("../../../packages").join(spec.atom)
}

pub fn text<'a>(value: &'a Json, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("fixture field {key} is not text: {value}"))
}

pub fn is_ok(fixture: &Json) -> bool {
    fixture["outcome"]["ok"].as_bool().expect("outcome.ok")
}

/// A step scenario: one file `{ case, env, nowMs?, steps: [...] }` replayed in order on one session.
pub fn is_step_scenario(file: &Json) -> bool {
    file.get("steps").is_some()
}

/// The single-record fixtures of the service (step scenarios are [`step_scenarios`]).
pub fn fixtures(spec: &ParitySpec) -> Vec<Json> {
    all_fixture_files(spec)
        .into_iter()
        .filter(|f| !is_step_scenario(f))
        .collect()
}

/// The step-scenario files of the service, in file-name order.
pub fn step_scenarios(spec: &ParitySpec) -> Vec<Json> {
    all_fixture_files(spec)
        .into_iter()
        .filter(is_step_scenario)
        .collect()
}

fn all_fixture_files(spec: &ParitySpec) -> Vec<Json> {
    let dir = root().join("fixtures").join(spec.fixtures_dir);
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
            if let Some(origin) = spec.base.map(|(_, origin)| origin).or(spec.fixture_origin) {
                text = text.replace("{base}", origin);
            }
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        })
        .collect()
}

pub fn fixture_named(spec: &ParitySpec, case: &str) -> Json {
    fixtures(spec)
        .into_iter()
        .find(|f| text(f, "case") == case)
        .unwrap_or_else(|| panic!("fixture {case}"))
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
    script: Mutex<VecDeque<(Json, ExchangeContract)>>,
    sent: Mutex<Vec<HttpRequest>>,
    os_script: Mutex<VecDeque<Json>>,
    os_sent: Mutex<Vec<OsCall>>,
    /// See [`Run::send_times`].
    send_times: Mutex<Vec<u64>>,
    entropy: Mutex<VecDeque<Vec<u8>>>,
    host: Arc<FixtureHost>,
    default_timeout: Duration,
    check_deadlines: bool,
    /// The fixture's `"beacon": "queue"`: the browser's sendBeacon queue accepts, so no request is replayed.
    beacon_queue: bool,
    /// The fixture's `"page": { hostname, referrer, screenWidth }`; absent means no page (`available: false`).
    page: Option<Json>,
}

impl<'a> Replay<'a> {
    fn new(
        spec: &'a ParitySpec,
        fixture: &Json,
        step: &ReplayStep,
        host: Arc<FixtureHost>,
        check_deadlines: bool,
    ) -> Arc<Self> {
        let recorded = fixture["exchanges"].as_array().expect("exchanges");
        let contracts = if step.exchanges.is_empty() {
            vec![ExchangeContract::default(); recorded.len()]
        } else {
            assert_eq!(
                step.exchanges.len(),
                recorded.len(),
                "{}: exchange contract count",
                text(fixture, "case")
            );
            step.exchanges.clone()
        };
        Arc::new(Self {
            spec,
            case: text(fixture, "case").to_string(),
            script: Mutex::new(recorded.iter().cloned().zip(contracts).collect()),
            sent: Mutex::new(Vec::new()),
            os_script: Mutex::new(
                fixture
                    .get("osExchanges")
                    .filter(|exchanges| !exchanges.is_null())
                    .map(|exchanges| {
                        exchanges
                            .as_array()
                            .expect("osExchanges is an array")
                            .iter()
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            os_sent: Mutex::new(Vec::new()),
            send_times: Mutex::new(Vec::new()),
            entropy: Mutex::new(step.entropy.iter().cloned().collect()),
            host,
            default_timeout: expected_timeout(spec, fixture),
            check_deadlines,
            beacon_queue: match fixture.get("beacon") {
                None => false,
                Some(Json::String(mode)) if mode == "queue" => true,
                Some(other) => panic!(
                    "{}: `beacon` must be \"queue\" or absent, not {other}",
                    text(fixture, "case")
                ),
            },
            page: fixture.get("page").cloned(),
        })
    }

    fn unused(&self) -> usize {
        self.script.lock().expect("script").len()
    }

    fn unused_os(&self) -> usize {
        self.os_script.lock().expect("os script").len()
    }

    fn answer_os(&self, op: OsOp, args: &[Value]) -> Result<Value, OsError> {
        let case = &self.case;
        let actual: Vec<Json> = args
            .iter()
            .map(|arg| by_value(serde_json::to_value(arg).expect("os arg json")))
            .collect();
        self.os_sent.lock().expect("os sent").push(OsCall {
            op,
            args: actual.clone(),
        });
        let exchange = self
            .os_script
            .lock()
            .expect("os script")
            .pop_front()
            .unwrap_or_else(|| {
                panic!(
                    "{case}: the atom made an unrecorded os ask: {} {actual:?}",
                    op.name()
                )
            });
        let recorded = &exchange["os"];
        assert_eq!(op.name(), text(recorded, "op"), "{case}: os op");
        let expected = recorded["args"]
            .as_array()
            .unwrap_or_else(|| panic!("{case}: os args must be an array"));
        assert!(
            json_matches(&Json::Array(expected.clone()), &Json::Array(actual.clone())),
            "{case}: {} args: expected {expected:?} got {actual:?}",
            op.name()
        );
        let answer = &exchange["answer"];
        match (answer.get("ok"), answer.get("error")) {
            (Some(value), None) => Ok(serde_json::from_value(value.clone())
                .unwrap_or_else(|e| panic!("{case}: os answer is not a value: {e}"))),
            (None, Some(error)) => {
                let kind = text(error, "kind");
                Err(OsError::new(
                    OsErrorKind::from_name(kind)
                        .unwrap_or_else(|| panic!("{case}: unknown os error kind {kind}")),
                    text(error, "message"),
                ))
            }
            _ => panic!("{case}: an os answer has `ok` or `error`, not both or neither: {answer}"),
        }
    }

    fn semantic(
        &self,
        headers: &[(String, String)],
        recorded: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
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
        let (exchange, contract) = self
            .script
            .lock()
            .expect("script")
            .pop_front()
            .unwrap_or_else(|| {
                panic!(
                    "{case}: the atom sent an unrecorded request: {} {}",
                    request.method, request.url
                )
            });
        let recorded = &exchange["request"];
        assert_eq!(request.method, text(recorded, "method"), "{case}: method");
        assert!(
            matches_recorded(text(recorded, "url"), &request.url),
            "{case}: url: expected {} got {}",
            text(recorded, "url"),
            request.url
        );
        let expected: BTreeMap<String, String> = recorded["headers"]
            .as_object()
            .expect("recorded headers")
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().expect("header text").to_string()))
            .collect();
        assert_eq!(
            self.semantic(&request.headers, &expected),
            expected,
            "{case}: headers"
        );
        let expected_body = contract.request_body.bytes(recorded);
        let body_matches = request.body == expected_body
            || match (
                std::str::from_utf8(&expected_body),
                std::str::from_utf8(&request.body),
            ) {
                (Ok(expected), Ok(actual)) => matches_recorded(expected, actual),
                _ => false,
            };
        assert!(body_matches, "{case}: request body bytes");
        if self.check_deadlines {
            let timeout = match contract.timeout {
                Deadline::Default => Some(self.default_timeout),
                Deadline::Exact(timeout) => timeout,
            };
            assert_eq!(request.timeout, timeout, "{case}: timeout");
        }
        let reply = &exchange["response"];
        let headers = reply["headers"]
            .as_object()
            .map(|h| {
                h.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().expect("header text").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(HttpResponse {
            status: reply["status"].as_u64().expect("status") as u16,
            headers,
            body: contract.response_body.bytes(reply),
        })
    }

    /// Only the requested fields are readable; the rest carry the no-page defaults.
    fn page_facts(&self, fields: &[PageFactField]) -> PageFacts {
        let Some(page) = &self.page else {
            return PageFacts {
                available: false,
                hostname: String::new(),
                referrer: String::new(),
                screen_width: None,
            };
        };
        let asked = |field: PageFactField| fields.contains(&field);
        PageFacts {
            available: true,
            hostname: if asked(PageFactField::Hostname) {
                page["hostname"].as_str().unwrap_or_default().to_string()
            } else {
                String::new()
            },
            referrer: if asked(PageFactField::Referrer) {
                page["referrer"].as_str().unwrap_or_default().to_string()
            } else {
                String::new()
            },
            screen_width: if asked(PageFactField::ScreenWidth) {
                page["screenWidth"].as_f64()
            } else {
                None
            },
        }
    }

    fn answer_io(&self, request: &IoRequest, store: &dyn EntitySource) -> IoAnswer {
        match request {
            IoRequest::Http {
                request, delay_ms, ..
            } => {
                // The host waits `delayMs` before it sends; on the virtual clock each wait follows the previous send.
                let mut times = self.send_times.lock().expect("send times");
                let previous = times.last().copied().unwrap_or(self.host.now() as u64);
                times.push(previous + delay_ms.unwrap_or(0));
                drop(times);
                IoAnswer::Http(self.answer(request))
            }
            IoRequest::Os { op, args, .. } => IoAnswer::Os(self.answer_os(*op, args)),
            IoRequest::Secret { name, principal } => {
                IoAnswer::Secret(self.host.secret_of(name, principal.as_deref()))
            }
            IoRequest::SecretSet {
                name,
                value,
                ttl_ms,
                principal,
            } => IoAnswer::SecretSet(self.host.store_secret_of(
                name,
                value,
                *ttl_ms,
                principal.as_deref(),
            )),
            IoRequest::SecretRotate {
                name,
                value,
                principal,
            } => IoAnswer::SecretRotate(self.host.rotate_secret_of(
                name,
                value,
                principal.as_deref(),
            )),
            IoRequest::CredentialStatus { name, principal } => {
                IoAnswer::CredentialStatus(self.host.secret_of(name, principal.as_deref()).map(
                    |value| orbital_core::credential_admin::CredentialStanding::of(credential_status_connected(value.as_deref(), principal.as_deref())),
                ))
            }
            IoRequest::CredentialForget { name, principal } => {
                IoAnswer::CredentialForget(self.host.forget_of(name, principal.as_deref()))
            }
            IoRequest::OAuth { ask, principal } => {
                IoAnswer::OAuth(self.host.oauth_of(ask, principal.as_deref()))
            }
            IoRequest::HttpBeacon { request, .. } if self.beacon_queue => {
                self.sent.lock().expect("sent").push(request.clone());
                IoAnswer::HttpBeacon(Ok(BeaconOutcome::Queued))
            }
            IoRequest::HttpBeacon { request, .. } => {
                IoAnswer::HttpBeacon(self.answer(request).map(|reply| BeaconOutcome::Fetched {
                    status: reply.status,
                }))
            }
            IoRequest::PageFacts { fields } => IoAnswer::PageFacts(Ok(self.page_facts(fields))),
            IoRequest::Random { bytes } => {
                let value = self
                    .entropy
                    .lock()
                    .expect("entropy")
                    .pop_front()
                    .unwrap_or_else(|| {
                        panic!("{}: unrecorded entropy ask for {bytes} bytes", self.case)
                    });
                assert_eq!(
                    value.len(),
                    usize::try_from(*bytes).expect("entropy count"),
                    "{}: entropy byte count",
                    self.case
                );
                IoAnswer::Random(Ok(value))
            }
            other => BlockingHost {
                source: store,
                service: None,
                substrate: None,
            }
            .answer(other),
        }
    }
}

#[cfg(test)]
pub fn probe_fixture_host(
    spec: &ParitySpec,
    fixture: &Json,
    requests: &[IoRequest],
) -> Vec<IoAnswer> {
    probe_fixture_host_with_authority(spec, fixture, FixtureAuthority::default(), requests)
}

#[cfg(test)]
pub fn probe_fixture_host_with_authority(
    spec: &ParitySpec,
    fixture: &Json,
    authority: FixtureAuthority,
    requests: &[IoRequest],
) -> Vec<IoAnswer> {
    let store = InMemoryEntityCache::new();
    let host = FixtureHost::new(spec, fixture, authority);
    let replay = Replay::new(spec, fixture, &ReplayStep::default(), host, false);
    requests
        .iter()
        .map(|request| replay.answer_io(request, &store))
        .collect()
}

fn expected_timeout(spec: &ParitySpec, fixture: &Json) -> Duration {
    let ms = spec
        .env_knobs
        .iter()
        .find(|(_, knob)| *knob == "timeoutMs")
        .and_then(|(variable, _)| fixture["env"][*variable].as_str())
        .and_then(|raw| match (raw.parse::<u64>(), spec.unreadable_knob) {
            (Ok(ms), _) => Some(ms),
            (Err(_), UnreadableKnob::UseDefault) => None,
            (Err(_), UnreadableKnob::Fail) => panic!(
                "{}: timeout env {raw:?} is not a number",
                text(fixture, "case")
            ),
        })
        .unwrap_or(spec.default_timeout_ms);
    Duration::from_millis(ms)
}

fn set_knob(json: &mut Json, knob: &str, value: &Json) -> bool {
    let mut patched = false;
    for orbital in json["orbitals"].as_array_mut().expect("orbitals") {
        for t in orbital["traits"].as_array_mut().into_iter().flatten() {
            if let Some(entry) = t["config"].get_mut(knob) {
                entry["default"] = value.clone();
                patched = true;
            }
        }
    }
    patched
}

fn require_knob(json: &mut Json, case: &str, knob: &str, value: &Json) -> Result<(), KnobError> {
    if set_knob(json, knob, value) {
        Ok(())
    } else {
        Err(KnobError::Undeclared {
            case: case.to_string(),
            knob: knob.to_string(),
        })
    }
}

/// The lowered atom with the fixture's knobs patched in. Knobs are patched before resolve because resolve folds a
/// knob's default into the effects.
pub fn patched_json(spec: &ParitySpec, fixture: &Json) -> Result<Json, KnobError> {
    let case = text(fixture, "case");
    let path = atom_path(spec);
    let schema = orbital_test_support::lower_lolo_file(&path)
        .unwrap_or_else(|e| panic!("lower {}: {e}", path.display()));
    let mut json = serde_json::to_value(&schema).expect("schema json");
    for (variable, knob) in spec.env_knobs {
        let Some(raw) = fixture["env"][*variable].as_str() else {
            continue;
        };
        let value = if *knob == "timeoutMs" {
            match (raw.parse::<u64>(), spec.unreadable_knob) {
                (Ok(ms), _) => json!(ms),
                (Err(_), UnreadableKnob::UseDefault) => continue,
                (Err(_), UnreadableKnob::Fail) => {
                    return Err(KnobError::Unreadable {
                        case: case.to_string(),
                        variable: (*variable).to_string(),
                        knob: (*knob).to_string(),
                        raw: raw.to_string(),
                    })
                }
            }
        } else {
            json!(raw)
        };
        require_knob(&mut json, case, knob, &value)?;
    }
    for (variable, knob) in spec.presence_knobs {
        let present = fixture["env"][*variable]
            .as_str()
            .is_some_and(|value| !value.is_empty());
        require_knob(&mut json, case, knob, &json!(present))?;
    }
    for (name, knob, value) in spec.case_knobs {
        if *name == case {
            require_knob(&mut json, case, knob, &value.json())?;
        }
    }
    if let Some((knob, origin)) = spec.base {
        require_knob(&mut json, case, knob, &json!(origin))?;
    }
    Ok(json)
}

fn lowered(spec: &ParitySpec, fixture: &Json) -> OrbitalSchema {
    let json = patched_json(spec, fixture).unwrap_or_else(|e| panic!("{e}"));
    let schema: OrbitalSchema = serde_json::from_value(json).expect("schema back from json");
    orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
    let path = atom_path(spec);
    let options = orbital_compiler::CompilerOptions::from_env();
    let (schema, _) =
        orbital_compiler::phases::lazy::resolve_program_with_path(schema, &options, Some(&path))
            .unwrap_or_else(|e| panic!("resolve {}: {e}", path.display()));
    schema
}

fn fixture_now(fixture: &Json) -> Option<f64> {
    let iso = fixture.get("now")?.as_str().unwrap_or_else(|| {
        panic!(
            "{}: `now` must be an ISO time string",
            text(fixture, "case")
        )
    });
    let at = chrono::DateTime::parse_from_rfc3339(iso)
        .unwrap_or_else(|e| panic!("{}: `now` {iso}: {e}", text(fixture, "case")))
        .timestamp_millis() as f64;
    Some(at)
}

pub struct ParitySession<'a> {
    spec: &'a ParitySpec,
    store: Arc<dyn EntitySource>,
    kernel: RuntimeKernel,
    host: Arc<FixtureHost>,
}

impl<'a> ParitySession<'a> {
    pub fn new(spec: &'a ParitySpec, fixture: &Json, authority: FixtureAuthority) -> Self {
        Self::with_schema(spec, lowered(spec, fixture), fixture, authority)
    }

    pub fn with_schema(
        spec: &'a ParitySpec,
        schema: OrbitalSchema,
        fixture: &Json,
        authority: FixtureAuthority,
    ) -> Self {
        let store: Arc<dyn EntitySource> = Arc::new(InMemoryEntityCache::new());
        let host = FixtureHost::new(spec, fixture, authority);
        let clock_host = host.clone();
        let clock: DispatchClock = Arc::new(move || clock_host.now());
        let kernel = RuntimeKernel::new(
            RuntimeEnvironment::Server,
            store.clone(),
            KernelConfig {
                clock: Some(clock),
                ..KernelConfig::default()
            },
        );
        kernel
            .register_schema(schema)
            .unwrap_or_else(|e| panic!("register {}: {e:?}", spec.atom));
        Self {
            spec,
            store,
            kernel,
            host,
        }
    }

    pub fn run(&mut self, fixture: &Json, step: ReplayStep) -> Run {
        self.run_inner(fixture, step, false)
    }

    /// A session for a step scenario: the atom lowered with the scenario's env, the fixture clock at its `nowMs`.
    pub fn for_scenario(spec: &'a ParitySpec, scenario: &Json) -> Self {
        let authority = FixtureAuthority {
            now_ms: scenario
                .get("nowMs")
                .map(|ms| ms.as_u64().expect("nowMs is a whole number")),
            ..FixtureAuthority::default()
        };
        Self::new(spec, scenario, authority)
    }

    /// Replays every step of `scenario` in order on THIS session (state carries over: pending grants, minted
    /// tokens, stored objects). A step may carry `advanceMs` (moves the clock on from the previous step) or `clockMs`
    /// (an absolute time), `randoms` (entropy, each a byte array or standard base64), `user` (the signed-in user).
    /// `{{N.field}}` in a later step's params, exchanges or outcome is the field of step N's emitted result.
    /// Returns each step's [`Run`]; nothing is asserted (see [`Self::check_steps`]).
    pub fn run_steps(&mut self, scenario: &Json) -> Vec<(Json, Run)> {
        self.steps(scenario, false)
    }

    /// [`Self::run_steps`], asserting every step like [`Self::check`] without the per-exchange deadline comparison
    /// (outcome and request tokens such as `<hex32>` match any generated value of that shape).
    pub fn check_steps(&mut self, scenario: &Json) -> Vec<(Json, Run)> {
        self.steps(scenario, true)
    }

    fn steps(&mut self, scenario: &Json, assert_steps: bool) -> Vec<(Json, Run)> {
        let case = text(scenario, "case").to_string();
        let mut clock = scenario
            .get("nowMs")
            .map(|ms| ms.as_u64().expect("nowMs is a whole number"));
        let mut emitted: Vec<Json> = Vec::new();
        let mut done = Vec::new();
        for (index, step) in scenario["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .enumerate()
        {
            let resolve = |value: &Json| resolve_placeholders(value, &emitted, &case, index);
            let at_ms = match (step.get("clockMs"), step.get("advanceMs")) {
                (Some(at), None) => Some(at.as_u64().expect("clockMs is a whole number")),
                (None, Some(by)) => {
                    clock.map(|now| now + by.as_u64().expect("advanceMs is a whole number"))
                }
                (None, None) => clock,
                (Some(_), Some(_)) => {
                    panic!("{case}#{index}: a step has clockMs or advanceMs, not both")
                }
            };
            clock = at_ms.or(clock);
            let fixture = json!({
                "case": format!("{case}#{index}"),
                "env": scenario["env"],
                "action": step["action"],
                "params": resolve(&step["params"]),
                "exchanges": resolve(&step["exchanges"]),
                "osExchanges": resolve(&step["osExchanges"]),
                "outcome": resolve(&step["outcome"]),
            });
            let entropy =
                step.get("randoms")
                    .and_then(Json::as_array)
                    .into_iter()
                    .flatten()
                    .map(|bytes| match bytes {
                        Json::String(text) => orbital_core::codec::base64_decode(text)
                            .unwrap_or_else(|e| {
                                panic!("{case}#{index}: randoms entry is not base64: {e}")
                            }),
                        Json::Array(items) => items
                            .iter()
                            .map(|b| {
                                u8::try_from(b.as_u64().expect("random byte")).expect("random byte")
                            })
                            .collect(),
                        other => panic!(
                            "{case}#{index}: randoms entry must be base64 or bytes, not {other}"
                        ),
                    })
                    .collect();
            let user = match step.get("user") {
                Some(user) => serde_json::from_value(user.clone()).expect("step user"),
                None => Value::Null,
            };
            let replay = ReplayStep {
                user,
                at_ms,
                entropy,
                ..Default::default()
            };
            let run = if assert_steps {
                self.check_without_deadlines(&fixture, replay)
            } else {
                self.run(&fixture, replay)
            };
            emitted.push(run.results.first().cloned().unwrap_or(Json::Null));
            done.push((fixture, run));
        }
        done
    }

    fn check_without_deadlines(&mut self, fixture: &Json, step: ReplayStep) -> Run {
        let run = self.run_inner(fixture, step, false);
        assert_run(self.spec, fixture, &run);
        run
    }

    /// The rows the session's store holds for `entity_type`, as JSON, for assertions on what an atom persisted.
    pub fn rows(&self, entity_type: &str) -> Vec<Json> {
        self.store
            .list(entity_type)
            .unwrap_or_else(|e| panic!("list {entity_type}: {e}"))
            .iter()
            .map(|row| by_value(serde_json::to_value(row.as_ref()).expect("row json")))
            .collect()
    }

    /// The credential value the host holds now (after any rotation), for assertions on a rotated token.
    pub fn held_credential(&self, name: &str, principal: Option<&str>) -> Option<String> {
        self.host
            .secret_of(name, principal)
            .expect("fixture credential")
    }

    /// The durable rotations the atom asked for so far, in order: `(credential, principal)`.
    pub fn rotations(&self) -> Vec<(String, Option<String>)> {
        self.host
            .rotations
            .lock()
            .expect("fixture rotations")
            .clone()
    }

    pub fn check(&mut self, fixture: &Json, step: ReplayStep) -> Run {
        let run = self.run_inner(fixture, step, true);
        assert_run(self.spec, fixture, &run);
        run
    }

    fn run_inner(&mut self, fixture: &Json, step: ReplayStep, check_deadlines: bool) -> Run {
        let events = events_of(self.spec, text(fixture, "action"));
        self.host.set_time(step.at_ms);
        self.kernel.set_current_user(step.user.clone());
        let replay = Replay::new(
            self.spec,
            fixture,
            &step,
            self.host.clone(),
            check_deadlines,
        );
        let payload: Value =
            serde_json::from_value(fixture["params"].clone()).expect("params as a payload");
        let progress = self
            .kernel
            .start_dispatch(
                self.spec.trait_name,
                None,
                events.input,
                payload,
                None,
                None,
            )
            .unwrap_or_else(|e| panic!("{}: dispatch: {e:?}", replay.case));
        let answer = |request: &IoRequest| replay.answer_io(request, self.store.as_ref());
        let outcomes = drive_blocking(&self.kernel, progress, &answer)
            .unwrap_or_else(|e| panic!("{}: drive: {e:?}", replay.case));
        let mut results = Vec::new();
        let mut failures = Vec::new();
        for (_, outcome) in &outcomes {
            assert_eq!(outcome.failure, None, "{}: an effect failed", replay.case);
            for effect in &outcome.effect_results {
                if let Effect::Emit { event, payload } = &effect.effect {
                    let value = by_value(serde_json::to_value(payload).expect("payload json"));
                    if event == events.result {
                        results.push(match events.data_field {
                            Some(field) => value.get(field).cloned().unwrap_or_else(|| {
                                panic!("{}: result has no `{field}` field: {value}", replay.case)
                            }),
                            None => value,
                        });
                    } else if events.failure.is_some_and(|f| f == event) {
                        failures.push(value);
                    }
                }
            }
        }
        let sent = replay.sent.lock().expect("sent").clone();
        let os_sent = replay.os_sent.lock().expect("os sent").clone();
        let send_times = replay.send_times.lock().expect("send times").clone();
        let unused_entropy = replay.entropy.lock().expect("entropy").len();
        Run {
            results,
            failures,
            sent,
            os_sent,
            unused_os: replay.unused_os(),
            unused: replay.unused(),
            unused_entropy,
            send_times,
        }
    }
}

/// `{{N.path}}` in a string is the text of step N's emitted result at `path`; a string that is only a placeholder
/// takes the value's own JSON type. A reference to a step that emitted nothing, or to a missing field, panics.
fn resolve_placeholders(value: &Json, emitted: &[Json], case: &str, step: usize) -> Json {
    match value {
        Json::String(text) if text.contains("{{") => {
            let lookup = |reference: &str| -> Json {
                let (n, path) = reference.split_once('.').unwrap_or((reference, ""));
                let n: usize = n
                    .parse()
                    .unwrap_or_else(|_| panic!("{case}#{step}: placeholder {{{{{reference}}}}}"));
                let mut found = emitted.get(n).unwrap_or_else(|| {
                    panic!("{case}#{step}: {{{{{reference}}}}} names a step that has not run")
                });
                for key in path.split('.').filter(|k| !k.is_empty()) {
                    found = found.get(key).unwrap_or_else(|| {
                        panic!("{case}#{step}: {{{{{reference}}}}}: step {n} emitted no `{key}`")
                    });
                }
                found.clone()
            };
            let trimmed = text.trim();
            if let Some(reference) = trimmed
                .strip_prefix("{{")
                .and_then(|t| t.strip_suffix("}}"))
            {
                if !reference.contains("{{") {
                    return lookup(reference);
                }
            }
            let mut out = String::new();
            let mut rest = text.as_str();
            while let Some(start) = rest.find("{{") {
                out.push_str(&rest[..start]);
                let end = rest[start..]
                    .find("}}")
                    .unwrap_or_else(|| panic!("{case}#{step}: unclosed placeholder"))
                    + start;
                match lookup(&rest[start + 2..end]) {
                    Json::String(s) => out.push_str(&s),
                    other => out.push_str(&other.to_string()),
                }
                rest = &rest[end + 2..];
            }
            out.push_str(rest);
            Json::String(out)
        }
        Json::Array(items) => Json::Array(
            items
                .iter()
                .map(|v| resolve_placeholders(v, emitted, case, step))
                .collect(),
        ),
        Json::Object(map) => Json::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), resolve_placeholders(v, emitted, case, step)))
                .collect(),
        ),
        other => other.clone(),
    }
}

pub fn run(spec: &ParitySpec, fixture: &Json) -> Run {
    ParitySession::new(spec, fixture, FixtureAuthority::default())
        .run(fixture, ReplayStep::default())
}

/// A generated value a recording cannot pin is written as a token in the recorded text: `<hex8>`, `<hex16>`,
/// `<hex32>`, `<hex64>` (exactly that many lowercase hex digits) or `<state>`, `<code_challenge>`, `<code_verifier>`
/// (one or more URL-safe base64 characters). Everything else, `<redacted>` included, is compared exactly.
#[derive(Debug)]
enum Part<'a> {
    Literal(&'a str),
    Hex(usize),
    UrlSafe,
}

fn parts(expected: &str) -> Vec<Part<'_>> {
    const TOKENS: [(&str, Option<usize>); 7] = [
        ("<hex8>", Some(8)),
        ("<hex16>", Some(16)),
        ("<hex32>", Some(32)),
        ("<hex64>", Some(64)),
        ("<state>", None),
        ("<code_challenge>", None),
        ("<code_verifier>", None),
    ];
    let mut out = Vec::new();
    let mut rest = expected;
    let mut literal_start = 0;
    let mut index = 0;
    while index < rest.len() {
        let hit = TOKENS
            .iter()
            .find(|(token, _)| rest[index..].starts_with(token));
        match hit {
            Some((token, width)) => {
                if index > literal_start {
                    out.push(Part::Literal(&rest[literal_start..index]));
                }
                out.push(width.map_or(Part::UrlSafe, Part::Hex));
                index += token.len();
                rest = &rest[index..];
                index = 0;
                literal_start = 0;
            }
            None => index += rest[index..].chars().next().map_or(1, char::len_utf8),
        }
    }
    if literal_start < rest.len() {
        out.push(Part::Literal(&rest[literal_start..]));
    }
    out
}

fn part_matches(parts: &[Part<'_>], actual: &str) -> bool {
    match parts.split_first() {
        None => actual.is_empty(),
        Some((Part::Literal(text), rest)) => actual
            .strip_prefix(text)
            .is_some_and(|tail| part_matches(rest, tail)),
        Some((Part::Hex(width), rest)) => {
            actual.len() >= *width
                && actual.is_char_boundary(*width)
                && actual[..*width]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && part_matches(rest, &actual[*width..])
        }
        Some((Part::UrlSafe, rest)) => {
            let run = actual
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
                .count();
            (1..=run)
                .rev()
                .any(|len| part_matches(rest, &actual[len..]))
        }
    }
}

/// Whether `actual` is what the recorded text `expected` describes.
pub fn matches_recorded(expected: &str, actual: &str) -> bool {
    expected == actual || part_matches(&parts(expected), actual)
}

/// [`matches_recorded`] across a JSON value: strings may carry tokens, everything else is equal or not.
pub fn json_matches(expected: &Json, actual: &Json) -> bool {
    match (expected, actual) {
        (Json::String(e), Json::String(a)) => matches_recorded(e, a),
        (Json::Array(e), Json::Array(a)) => {
            e.len() == a.len() && e.iter().zip(a).all(|(e, a)| json_matches(e, a))
        }
        (Json::Object(e), Json::Object(a)) => {
            e.len() == a.len()
                && e.iter()
                    .all(|(k, v)| a.get(k).is_some_and(|actual| json_matches(v, actual)))
        }
        _ => expected == actual,
    }
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
    ParitySession::new(spec, fixture, FixtureAuthority::default())
        .check(fixture, ReplayStep::default());
}

fn assert_run(spec: &ParitySpec, fixture: &Json, run: &Run) {
    let case = text(fixture, "case");
    assert_eq!(
        run.unused, 0,
        "{case}: recorded requests the atom never sent"
    );
    assert_eq!(
        run.sent.len(),
        fixture["exchanges"].as_array().expect("exchanges").len(),
        "{case}: request count"
    );
    assert_eq!(
        run.unused_os, 0,
        "{case}: recorded os asks the atom never made"
    );
    assert_eq!(run.unused_entropy, 0, "{case}: unused entropy");
    if is_ok(fixture) {
        assert!(
            run.failures.is_empty(),
            "{case}: unexpected failure event {:?}",
            run.failures
        );
        let expected = by_value(fixture["outcome"]["data"].clone());
        assert!(
            run.results.len() == 1 && json_matches(&expected, &run.results[0]),
            "{case}: emitted result: expected {expected} got {:?}",
            run.results
        );
    } else {
        let outcome = &fixture["outcome"];
        let mut expected = json!({"code": outcome["code"]});
        expected[spec.failure_message_field] = outcome["message"].clone();
        for (field, value) in (spec.failure_extras)(fixture) {
            expected[field] = value;
        }
        assert!(
            run.results.is_empty(),
            "{case}: unexpected result event {:?}",
            run.results
        );
        assert!(
            run.failures.len() == 1 && json_matches(&expected, &run.failures[0]),
            "{case}: emitted failure: expected {expected} got {:?}",
            run.failures
        );
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
    for fixture in fixtures(spec)
        .iter()
        .filter(|f| cases.contains(&text(f, "case")))
    {
        assert!(
            fixture["exchanges"]
                .as_array()
                .expect("exchanges")
                .is_empty(),
            "{}",
            text(fixture, "case")
        );
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
    assert_ne!(
        url,
        text(&fixture["exchanges"][0]["request"], "url"),
        "the control changed nothing"
    );
    fixture["exchanges"][0]["request"]["url"] = json!(url);
    check(spec, &fixture);
}

/// Control: a fixture whose recorded outcome data at `key` is replaced must fail the replay on the emitted result.
pub fn drifted_outcome_control(spec: &ParitySpec, case: &str, key: &str, value: Json) {
    let mut fixture = fixture_named(spec, case);
    assert_ne!(
        fixture["outcome"]["data"][key], value,
        "the control changed nothing"
    );
    fixture["outcome"]["data"][key] = value;
    check(spec, &fixture);
}

/// Fixtures hold only test values: secret-named env (`*_KEY`, `*_TOKEN`, `*_SECRET`) is `test-` prefixed or empty, secret headers are redacted.
pub fn assert_no_secrets(spec: &ParitySpec) {
    assert_no_secrets_allowing(spec, &[]);
}

/// [`assert_no_secrets`] for a service whose recordings need a real-looking throwaway value in a secret-named env
/// var (a P-256 test keypair, a base64 service-account key). Only the exact `allowed` values pass instead of the
/// `test-` prefix, and none of them may appear in the atom's source; every other check is unchanged.
pub fn assert_no_secrets_allowing(spec: &ParitySpec, allowed: &[&str]) {
    let mut all = fixtures(spec);
    all.extend(step_scenarios(spec));
    for fixture in &all {
        let case = text(fixture, "case");
        for (name, value) in fixture["env"].as_object().expect("env") {
            let known = spec.env_knobs.iter().any(|(v, _)| v == name)
                || spec.secret_env.contains(&name.as_str());
            let secretish = ["_KEY", "_TOKEN", "_SECRET", "_SECRET_ID"]
                .iter()
                .any(|s| name.ends_with(s));
            assert!(known || !secretish, "{case}: unexpected secret env {name}");
            // A credential slot may carry an identifier (an account SID as the basic-auth user); only
            // secret-named vars must hold test values.
            if secretish {
                let v = value.as_str().expect("env text");
                assert!(
                    v.starts_with("test-") || v.is_empty() || allowed.contains(&v),
                    "{case}: {name}"
                );
            }
        }
        let mut exchanges: Vec<&Json> = fixture["exchanges"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        for step in fixture["steps"].as_array().into_iter().flatten() {
            exchanges.extend(step["exchanges"].as_array().expect("step exchanges"));
        }
        assert!(
            is_step_scenario(fixture) || fixture["exchanges"].is_array(),
            "{case}: exchanges"
        );
        for exchange in exchanges {
            for header in SECRET_HEADERS {
                if let Some(value) = exchange["request"]["headers"].get(header) {
                    assert_eq!(value, REDACTED, "{case}: {header}");
                }
            }
        }
    }
    let source = std::fs::read_to_string(atom_path(spec)).expect("read the atom");
    for value in allowed.iter().filter(|value| !value.is_empty()) {
        assert!(
            !source.contains(value),
            "an allowed test secret appears in the atom"
        );
    }
    assert_eq!(
        source.contains(": secret"),
        !spec.secret_env.is_empty(),
        "the atom's secret knobs must match the spec's secret_env"
    );
}
