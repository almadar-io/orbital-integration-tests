// H.6 parity for postgres: every recorded scenario of `fixtures/postgres`, replayed through the kernel against
// `std-postgres.lolo`, makes the `os/*` asks the kernel's postgres driver makes (connect, startup, login, one
// Parse/Bind/Describe/Execute/Sync send, Terminate, close) and emits the recorded outcome. The scenarios were recorded
// against a real Postgres 16 server by `fixtures/postgres/record.py`, an independent client of the wire protocol, so the
// driver's bytes (including the SCRAM-SHA-256 proof for a fixed client nonce) are the bytes a real server accepted. The
// ones marked `synthetic` hand-write a host answer a live server cannot be made to give. A scenario that sets
// `PG_PASSWORD` in its env turns the `authenticated` knob on; role, database, TLS and timeout cases patch their knobs.
// The live test (`--ignored`) runs the same atom against a Postgres at `PG_LIVE_PORT` (default 15432).

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, KnobValue, ParitySession, ParitySpec};
use serde_json::{json, Value};

fn failure_extras(fixture: &Value) -> Vec<(&'static str, Value)> {
    vec![("kind", fixture["outcome"]["kind"].clone())]
}

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/store/atoms/std-postgres.lolo",
    trait_name: "PostgresClient",
    fixtures_dir: "postgres",
    events: &[ActionEvents { action: "query", input: "PG_QUERY", result: "PG_RESULT", failure: Some("PG_FAILED"), data_field: None }],
    secret_env: &["PG_PASSWORD"],
    presence_knobs: &[("PG_PASSWORD", "authenticated")],
    fixture_origin: Some("postgres://postgres.example.test:5432"),
    case_knobs: &[
        ("select-from-another-database", "database", KnobValue::Text("other")),
        ("statement-timeout-fails-with-57014", "statementTimeoutMs", KnobValue::Number(200)),
        ("missing-database-fails-at-startup", "database", KnobValue::Text("nope_db")),
        ("md5-login", "user", KnobValue::Text("md5user")),
        ("cleartext-login", "user", KnobValue::Text("clearuser")),
        ("trusted-login-without-a-password", "user", KnobValue::Text("trustuser")),
        ("read-timeout-fails", "user", KnobValue::Text("trustuser")),
        ("md5-login", "database", KnobValue::Text("postgres")),
        ("cleartext-login", "database", KnobValue::Text("postgres")),
        ("trusted-login-without-a-password", "database", KnobValue::Text("postgres")),
        ("read-timeout-fails", "database", KnobValue::Text("postgres")),
        ("tls-refused-by-the-server-fails", "tls", KnobValue::Bool(true)),
    ],
    failure_extras,
    ..ParitySpec::DEFAULT
};

fn scenario(case: &str) -> Value {
    lolo_parity::step_scenarios(&SPEC)
        .into_iter()
        .find(|s| lolo_parity::text(s, "case") == case)
        .unwrap_or_else(|| panic!("scenario {case}"))
}

fn replay(case: &str) -> Vec<(Value, lolo_parity::Run)> {
    let scenario = scenario(case);
    ParitySession::for_scenario(&SPEC, &scenario).check_steps(&scenario)
}

fn op_names(run: &lolo_parity::Run) -> Vec<&'static str> {
    run.os_sent.iter().map(|call| call.op.name()).collect()
}

#[test]
fn every_recorded_scenario_replays_to_its_recorded_asks_and_outcome() {
    let all = lolo_parity::step_scenarios(&SPEC);
    assert!(all.len() >= 28, "the recorded scenarios: {}", all.len());
    for scenario in &all {
        ParitySession::for_scenario(&SPEC, scenario).check_steps(scenario);
    }
}

#[test]
fn a_scram_login_draws_one_nonce_and_runs_the_statement_on_the_same_connection() {
    let runs = replay("select-rows-by-parameter");
    let ops = op_names(&runs[0].1);
    assert_eq!(ops.first(), Some(&"os/tcp-connect"));
    assert_eq!(ops.iter().filter(|op| **op == "os/send").count(), 5, "startup, SASL initial, SASL response, statement, terminate: {ops:?}");
    assert_eq!(ops.last(), Some(&"os/close"));
    assert_eq!(runs[0].1.sent.len(), 0, "postgres speaks no http");
}

#[test]
fn a_trusted_login_sends_no_credential_and_names_no_nonce() {
    let runs = replay("trusted-login-without-a-password");
    assert_eq!(runs[0].1.unused_entropy, 0);
    let ops = op_names(&runs[0].1);
    assert_eq!(ops.iter().filter(|op| **op == "os/send").count(), 3, "startup, statement, terminate");
}

#[test]
fn control_a_wrong_password_never_sends_the_statement() {
    let runs = replay("scram-wrong-password-fails");
    let ops = op_names(&runs[0].1);
    assert_eq!(ops.iter().filter(|op| **op == "os/send").count(), 3, "startup, SASL initial, SASL response: {ops:?}");
    assert_eq!(runs[0].1.failures[0]["code"], json!("28P01"));
}

#[test]
fn control_a_refused_statement_makes_no_ask_at_all() {
    for case in [
        "delete-is-refused-before-any-ask",
        "second-statement-is-refused-before-any-ask",
        "select-into-is-refused-before-any-ask",
        "select-for-update-is-refused-before-any-ask",
        "empty-statement-is-refused-before-any-ask",
    ] {
        let runs = replay(case);
        assert!(runs[0].1.os_sent.is_empty(), "{case}");
        assert_eq!(runs[0].1.failures[0]["kind"], json!("refused"), "{case}");
    }
}

#[test]
fn control_a_failed_connect_makes_no_further_ask() {
    let runs = replay("connect-not-granted-fails");
    assert_eq!(runs[0].1.os_sent.len(), 1);
}

#[test]
fn a_tls_request_to_a_server_without_tls_ends_the_call_after_one_byte() {
    let runs = replay("tls-refused-by-the-server-fails");
    assert_eq!(op_names(&runs[0].1), ["os/tcp-connect", "os/send", "os/recv", "os/close"]);
}

#[test]
#[should_panic(expected = "args")]
fn control_a_drifted_statement_fails_the_replay() {
    let mut scenario = scenario("select-generate-series");
    scenario["steps"][0]["osExchanges"][3]["os"]["args"][1] = json!("AAAA");
    ParitySession::for_scenario(&SPEC, &scenario).check_steps(&scenario);
}

#[test]
#[should_panic(expected = "emitted result")]
fn control_a_drifted_outcome_fails_the_replay() {
    let mut scenario = scenario("select-generate-series");
    scenario["steps"][0]["outcome"]["data"]["rowCount"] = json!(4);
    ParitySession::for_scenario(&SPEC, &scenario).check_steps(&scenario);
}

#[test]
fn the_fixtures_hold_only_test_values() {
    lolo_parity::assert_no_secrets(&SPEC);
}

#[test]
#[ignore = "needs a Postgres 16 at 127.0.0.1:PG_LIVE_PORT (default 15432) with the roles of fixtures/postgres/record.py"]
fn live_postgres_runs_queries_end_to_end() {
    let port = std::env::var("PG_LIVE_PORT").unwrap_or_else(|_| "15432".to_string());
    live::round_trip("127.0.0.1", &port);
}

mod live {
    use super::*;
    use orbital_core::effects::Effect;
    use orbital_core::runtime::os_host::{answer_os, OsGrants, OsHandles, OsHost};
    use orbital_core::runtime::resumable::{drive_blocking, BlockingHost, IoAnswer, IoRequest};
    use orbital_core::runtime::{DispatchClock, InMemoryEntityCache, KernelConfig, RuntimeKernel};
    use orbital_core::{OrbitalSchema, RuntimeEnvironment, Value as KernelValue};
    use std::sync::Arc;

    fn schema(host: &str, port: &str, user: &str) -> OrbitalSchema {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(SPEC.atom);
        let schema = orbital_test_support::lower_lolo_file(&path).unwrap_or_else(|e| panic!("lower: {e}"));
        let mut json = serde_json::to_value(&schema).expect("schema json");
        json["egress"] = json!([format!("postgres://{host}:{port}")]);
        for orbital in json["orbitals"].as_array_mut().expect("orbitals") {
            for t in orbital["traits"].as_array_mut().into_iter().flatten() {
                for (knob, value) in [("origin", json!(format!("postgres://{host}:{port}"))), ("user", json!(user)), ("database", json!("postgres")), ("authenticated", json!(false))] {
                    if let Some(entry) = t["config"].get_mut(knob) {
                        entry["default"] = value;
                    }
                }
            }
        }
        let schema: OrbitalSchema = serde_json::from_value(json).expect("schema back from json");
        orbital_compiler::loader::register_source_lowerer(orbital_test_support::lower_lolo_file);
        let options = orbital_compiler::CompilerOptions::from_env();
        orbital_compiler::phases::lazy::resolve_program_with_path(schema, &options, Some(&path))
            .unwrap_or_else(|e| panic!("resolve: {e}"))
            .0
    }

    fn query(kernel: &RuntimeKernel, os: &OsHost<'_>, sql: &str, params: Value) -> Vec<(String, Value)> {
        let payload: KernelValue = serde_json::from_value(json!({ "sql": sql, "params": params })).expect("payload");
        let progress = kernel
            .start_dispatch(SPEC.trait_name, None, "PG_QUERY", payload, None, None)
            .unwrap_or_else(|e| panic!("dispatch: {e:?}"));
        let store = InMemoryEntityCache::new();
        let answer = |request: &IoRequest| match request {
            IoRequest::Os { .. } => answer_os(Some(os), request),
            IoRequest::Random { bytes } => IoAnswer::Random(Ok(vec![9u8; usize::try_from(*bytes).expect("count")])),
            other => BlockingHost { source: &store, service: None, substrate: None }.answer(other),
        };
        let outcomes = drive_blocking(kernel, progress, &answer).unwrap_or_else(|e| panic!("drive: {e:?}"));
        let mut emitted = Vec::new();
        for (_, outcome) in &outcomes {
            for effect in &outcome.effect_results {
                if let Effect::Emit { event, payload } = &effect.effect {
                    emitted.push((event.clone(), serde_json::to_value(payload).expect("payload json")));
                }
            }
        }
        emitted
    }

    fn whole_numbers(value: Value) -> Value {
        match value {
            Value::Number(n) => match n.as_f64() {
                Some(f) if f.fract() == 0.0 && f.abs() < 9.007_199_254_740_992e15 => json!(f as i64),
                _ => Value::Number(n),
            },
            Value::Array(items) => Value::Array(items.into_iter().map(whole_numbers).collect()),
            Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, whole_numbers(v))).collect()),
            other => other,
        }
    }

    fn outcome(emitted: &[(String, Value)], event: &str) -> Value {
        emitted
            .iter()
            .find(|(name, _)| name == event)
            .map(|(_, p)| whole_numbers(p.clone()))
            .unwrap_or_else(|| panic!("no {event} among {emitted:?}"))
    }

    pub fn round_trip(host: &str, port: &str) {
        let grants: OsGrants = serde_json::from_value(json!({ "net": [{ "target": format!("{host}:{port}"), "kinds": ["tcp"] }] })).expect("grants");
        let handles = OsHandles::default();
        let on_event = |_: Option<&orbital_core::runtime::resumable::CallSite>, _: KernelValue| {};
        let os = OsHost { grants: &grants, handles: &handles, on_event: &on_event };
        let store: Arc<dyn orbital_core::runtime::EntitySource> = Arc::new(InMemoryEntityCache::new());
        let clock: DispatchClock = Arc::new(|| orbital_core::time_util::now_millis_f64());
        let kernel = RuntimeKernel::new(RuntimeEnvironment::Server, store, KernelConfig { clock: Some(clock), ..KernelConfig::default() });
        kernel.register_schema(schema(host, port, "trustuser")).expect("register");
        let rows = query(&kernel, &os, "SELECT id, name FROM users WHERE id >= $1 ORDER BY id", json!([2]));
        let result = outcome(&rows, "PG_RESULT");
        assert_eq!(result["rows"], json!([{ "id": 2, "name": "bob" }, { "id": 3, "name": "carol" }]));
        assert_eq!(result["rowCount"], json!(2));
        let arrays = query(&kernel, &os, "SELECT ARRAY[1,2,NULL]::int4[] a, NULL::text b", json!([]));
        assert_eq!(outcome(&arrays, "PG_RESULT")["rows"], json!([{ "a": [1, 2, null], "b": null }]));
        let refused = query(&kernel, &os, "DELETE FROM users", json!([]));
        let failure = outcome(&refused, "PG_FAILED");
        assert_eq!(failure["kind"], json!("refused"));
        let missing = query(&kernel, &os, "SELECT * FROM nope_table", json!([]));
        let failure = outcome(&missing, "PG_FAILED");
        assert_eq!((failure["kind"].as_str(), failure["code"].as_str()), (Some("postgres"), Some("42P01")));
    }
}
