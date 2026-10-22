// H.6 parity for redis: every recorded fixture of `fixtures/redis`, replayed through the kernel against `std-redis.lolo`,
// makes the `os/*` asks the kernel's redis driver makes (connect, one send per pipeline, a recv per read, close) and
// emits the recorded outcome. The fixtures were recorded against real Redis 7 servers by `fixtures/redis/record.py`;
// the ones marked `synthetic` hand-write a host answer a live server cannot be made to give (a refused grant, a
// closed port, a read timeout) or cut a real reply across two reads. a fixture that sets `REDIS_PASSWORD` in its env turns the
// `authenticated` knob on and the fixture env answers the credential; the ACL user and database cases
// patch `user` and `db`. The live test (`--ignored`) runs the same atom against a Redis at `REDIS_LIVE_URL`.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, KnobValue, ParitySession, ParitySpec};
use serde_json::{json, Value};

fn failure_extras(fixture: &Value) -> Vec<(&'static str, Value)> {
    vec![
        ("action", fixture["action"].clone()),
        ("kind", fixture["outcome"]["kind"].clone()),
    ]
}

const FAILED: Option<&str> = Some("REDIS_FAILED");

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/store/atoms/std-redis.lolo",
    trait_name: "RedisClient",
    fixtures_dir: "redis",
    events: &[
        ActionEvents { action: "get", input: "REDIS_GET", result: "REDIS_GOT", failure: FAILED, data_field: None },
        ActionEvents { action: "set", input: "REDIS_SET", result: "REDIS_SET_DONE", failure: FAILED, data_field: None },
        ActionEvents { action: "delete", input: "REDIS_DELETE", result: "REDIS_DELETED", failure: FAILED, data_field: None },
        ActionEvents { action: "increment", input: "REDIS_INCREMENT", result: "REDIS_INCREMENTED", failure: FAILED, data_field: None },
        ActionEvents { action: "expire", input: "REDIS_EXPIRE", result: "REDIS_EXPIRED", failure: FAILED, data_field: None },
        ActionEvents { action: "publish", input: "REDIS_PUBLISH", result: "REDIS_PUBLISHED", failure: FAILED, data_field: None },
        ActionEvents { action: "lock", input: "REDIS_LOCK", result: "REDIS_LOCKED", failure: FAILED, data_field: None },
        ActionEvents { action: "unlock", input: "REDIS_UNLOCK", result: "REDIS_UNLOCKED", failure: FAILED, data_field: None },
    ],
    secret_env: &["REDIS_PASSWORD"],
    presence_knobs: &[("REDIS_PASSWORD", "authenticated")],
    fixture_origin: Some("redis://redis.example.test:6379"),
    case_knobs: &[
        ("get-as-an-acl-user-in-database-3", "user", KnobValue::Text("app")),
        ("get-as-an-acl-user-in-database-3", "db", KnobValue::Number(3)),
        ("get-from-an-out-of-range-database-fails", "db", KnobValue::Number(99)),
    ],
    failure_extras,
    ..ParitySpec::DEFAULT
};

fn check(fixture: &Value) -> lolo_parity::Run {
    ParitySession::new(&SPEC, fixture, lolo_parity::FixtureAuthority::default())
        .check(fixture, lolo_parity::ReplayStep::default())
}

fn named(case: &str) -> Value {
    lolo_parity::fixture_named(&SPEC, case)
}

#[test]
fn every_recorded_case_replays_to_its_recorded_asks_and_outcome() {
    let all = lolo_parity::fixtures(&SPEC);
    assert!(all.len() >= 26, "the recorded cases: {}", all.len());
    for fixture in &all {
        check(fixture);
    }
}

#[test]
fn the_lock_cycle_takes_refuses_and_releases_a_lock_on_one_session() {
    let scenarios = lolo_parity::step_scenarios(&SPEC);
    let [scenario] = scenarios.as_slice() else { panic!("one step scenario, got {}", scenarios.len()) };
    let mut session = ParitySession::for_scenario(&SPEC, scenario);
    let runs = session.check_steps(scenario);
    assert_eq!(runs.len(), 5);
    let lock = &runs[0].1.results[0];
    assert_eq!(lock["acquired"], json!(true));
    assert_eq!(lock["lockId"], scenario["lockIds"][0]);
    assert_eq!(runs[1].1.results[0]["acquired"], json!(false), "a held lock is not taken twice");
}

#[test]
fn a_get_makes_one_connection_with_one_send_and_one_read() {
    let run = check(&named("get-hit"));
    let ops: Vec<&str> = run.os_sent.iter().map(|call| call.op.name()).collect();
    assert_eq!(ops, ["os/tcp-connect", "os/send", "os/recv", "os/close"]);
    assert!(run.sent.is_empty(), "redis speaks no http");
}

#[test]
fn a_password_and_a_database_ride_one_pipeline_ahead_of_the_command() {
    let run = check(&named("get-as-an-acl-user-in-database-3"));
    let ops: Vec<&str> = run.os_sent.iter().map(|call| call.op.name()).collect();
    assert_eq!(ops, ["os/tcp-connect", "os/send", "os/recv", "os/send", "os/recv", "os/close"]);
}

#[test]
fn a_reply_cut_across_two_reads_asks_for_the_second_read() {
    let run = check(&named("get-reply-split-across-reads"));
    let reads = run.os_sent.iter().filter(|call| call.op.name() == "os/recv").count();
    assert_eq!(reads, 2);
}

#[test]
fn control_a_wrong_password_never_sends_the_command() {
    let run = check(&named("get-wrong-password-fails"));
    let sends = run.os_sent.iter().filter(|call| call.op.name() == "os/send").count();
    assert_eq!(sends, 1, "only the AUTH pipeline goes out");
    assert_eq!(run.failures[0]["code"], json!("WRONGPASS"));
}

#[test]
fn control_a_failed_connect_makes_no_further_ask() {
    let run = check(&named("get-connect-not-granted-fails"));
    assert_eq!(run.os_sent.len(), 1);
    assert_eq!(run.failures[0]["kind"], json!("refused"));
}

#[test]
fn control_a_validation_refusal_makes_no_ask_at_all() {
    for case in ["get-empty-key-fails", "publish-empty-channel-fails", "unlock-without-a-lock-id-fails"] {
        let run = check(&named(case));
        assert!(run.os_sent.is_empty(), "{case}");
        assert_eq!(run.failures[0]["kind"], json!("refused"), "{case}");
    }
}

#[test]
#[should_panic(expected = "args")]
fn control_a_drifted_command_fails_the_replay() {
    let mut fixture = named("set-with-ttl");
    fixture["osExchanges"][1]["os"]["args"][1] = json!("KjENCiQ0DQpQSU5HDQo=");
    check(&fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
fn control_a_drifted_outcome_fails_the_replay() {
    let mut fixture = named("get-hit");
    fixture["outcome"]["data"]["value"] = json!("bob");
    check(&fixture);
}

#[test]
fn the_fixtures_hold_only_test_values() {
    lolo_parity::assert_no_secrets(&SPEC);
}

#[test]
#[ignore = "needs a Redis at REDIS_LIVE_URL (default 127.0.0.1:16380) and the kernel's native os host"]
fn live_redis_round_trips_every_action() {
    let host = std::env::var("REDIS_LIVE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("REDIS_LIVE_PORT").unwrap_or_else(|_| "16380".to_string());
    live::round_trip(&host, &port);
}

mod live {
    use super::*;
    use orbital_core::effects::Effect;
    use orbital_core::runtime::os_host::{answer_os, OsGrants, OsHandles, OsHost};
    use orbital_core::runtime::resumable::{drive_blocking, BlockingHost, IoAnswer, IoRequest};
    use orbital_core::runtime::{DispatchClock, InMemoryEntityCache, KernelConfig, RuntimeKernel};
    use orbital_core::{OrbitalSchema, RuntimeEnvironment, Value as KernelValue};
    use std::sync::Arc;

    fn schema(host: &str, port: &str) -> OrbitalSchema {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(SPEC.atom);
        let schema = orbital_test_support::lower_lolo_file(&path).unwrap_or_else(|e| panic!("lower: {e}"));
        let mut json = serde_json::to_value(&schema).expect("schema json");
        json["egress"] = json!([format!("redis://{host}:{port}")]);
        for orbital in json["orbitals"].as_array_mut().expect("orbitals") {
            for t in orbital["traits"].as_array_mut().into_iter().flatten() {
                if let Some(entry) = t["config"].get_mut("origin") {
                    entry["default"] = json!(format!("redis://{host}:{port}"));
                }
                if let Some(entry) = t["config"].get_mut("authenticated") {
                    entry["default"] = json!(false);
                }
            }
        }
        let schema: OrbitalSchema = serde_json::from_value(json).expect("schema back from json");
        orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
        let options = orbital_compiler::CompilerOptions::from_env();
        orbital_compiler::phases::lazy::resolve_program_with_path(schema, &options, Some(&path))
            .unwrap_or_else(|e| panic!("resolve: {e}"))
            .0
    }

    fn dispatch(kernel: &RuntimeKernel, grants: &OsGrants, handles: &OsHandles, event: &str, payload: Json) -> Vec<(String, Json)> {
        let payload: KernelValue = serde_json::from_value(payload).expect("payload");
        let progress = kernel
            .start_dispatch(SPEC.trait_name, None, event, payload, None, None)
            .unwrap_or_else(|e| panic!("{event}: dispatch: {e:?}"));
        let store = InMemoryEntityCache::new();
        let on_event = |_: Option<&orbital_core::runtime::resumable::CallSite>, _: KernelValue| {};
        let os = OsHost { grants, handles, on_event: &on_event };
        let answer = |request: &IoRequest| match request {
            IoRequest::Os { .. } => answer_os(Some(&os), request),
            IoRequest::Random { bytes } => IoAnswer::Random(Ok(vec![7u8; usize::try_from(*bytes).expect("count")])),
            other => BlockingHost { source: &store, service: None, substrate: None }.answer(other),
        };
        let outcomes = drive_blocking(kernel, progress, &answer).unwrap_or_else(|e| panic!("{event}: drive: {e:?}"));
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

    type Json = serde_json::Value;

    fn outcome(emitted: &[(String, Json)], event: &str) -> Json {
        emitted
            .iter()
            .find(|(name, _)| name == event)
            .map(|(_, payload)| payload.clone())
            .unwrap_or_else(|| panic!("no {event} among {emitted:?}"))
    }

    pub fn round_trip(host: &str, port: &str) {
        let grants: OsGrants = serde_json::from_value(json!({ "net": [{ "target": format!("{host}:{port}"), "kinds": ["tcp"] }] }))
            .expect("grants");
        let handles = OsHandles::default();
        let store: Arc<dyn orbital_core::runtime::EntitySource> = Arc::new(InMemoryEntityCache::new());
        let clock: DispatchClock = Arc::new(|| orbital_core::time_util::now_millis_f64());
        let kernel = RuntimeKernel::new(
            RuntimeEnvironment::Server,
            store,
            KernelConfig { clock: Some(clock), ..KernelConfig::default() },
        );
        kernel.register_schema(schema(host, port)).expect("register");
        let key = format!("live:{}", std::process::id());
        let set = dispatch(&kernel, &grants, &handles, "REDIS_SET", json!({ "key": key, "value": "one", "ttl": 30 }));
        assert_eq!(outcome(&set, "REDIS_SET_DONE")["ok"], json!(true));
        let got = dispatch(&kernel, &grants, &handles, "REDIS_GET", json!({ "key": key }));
        assert_eq!(outcome(&got, "REDIS_GOT")["value"], json!("one"));
        let counter = format!("{key}:count");
        let first = dispatch(&kernel, &grants, &handles, "REDIS_INCREMENT", json!({ "key": counter, "by": 4 }));
        assert_eq!(outcome(&first, "REDIS_INCREMENTED")["value"], json!(4.0));
        let expired = dispatch(&kernel, &grants, &handles, "REDIS_EXPIRE", json!({ "key": counter, "ttl": 30 }));
        assert_eq!(outcome(&expired, "REDIS_EXPIRED")["set"], json!(true));
        let published = dispatch(&kernel, &grants, &handles, "REDIS_PUBLISH", json!({ "channel": "live", "message": "hi" }));
        assert_eq!(outcome(&published, "REDIS_PUBLISHED")["receivers"], json!(0.0));
        let lock_key = format!("{key}:lock");
        let locked = dispatch(&kernel, &grants, &handles, "REDIS_LOCK", json!({ "key": lock_key, "ttl": 5000 }));
        let taken = outcome(&locked, "REDIS_LOCKED");
        assert_eq!(taken["acquired"], json!(true));
        let id = taken["lockId"].as_str().expect("lock id").to_string();
        let again = dispatch(&kernel, &grants, &handles, "REDIS_LOCK", json!({ "key": lock_key }));
        assert_eq!(outcome(&again, "REDIS_LOCKED")["acquired"], json!(false), "a held lock is not taken twice");
        let wrong = dispatch(&kernel, &grants, &handles, "REDIS_UNLOCK", json!({ "key": lock_key, "lockId": "not-the-holder" }));
        assert_eq!(outcome(&wrong, "REDIS_UNLOCKED")["released"], json!(false));
        let released = dispatch(&kernel, &grants, &handles, "REDIS_UNLOCK", json!({ "key": lock_key, "lockId": id }));
        assert_eq!(outcome(&released, "REDIS_UNLOCKED")["released"], json!(true));
        let removed = dispatch(&kernel, &grants, &handles, "REDIS_DELETE", json!({ "key": key }));
        assert_eq!(outcome(&removed, "REDIS_DELETED")["deleted"], json!(true));
        let missing = dispatch(&kernel, &grants, &handles, "REDIS_GET", json!({ "key": key }));
        assert_eq!(outcome(&missing, "REDIS_GOT")["value"], Json::Null);
        let cleanup = dispatch(&kernel, &grants, &handles, "REDIS_DELETE", json!({ "key": counter }));
        assert_eq!(outcome(&cleanup, "REDIS_DELETED")["deleted"], json!(true));
        let wrongtype = dispatch(&kernel, &grants, &handles, "REDIS_SET", json!({ "key": counter, "value": "x" }));
        assert_eq!(outcome(&wrongtype, "REDIS_SET_DONE")["ok"], json!(true));
        let broken = dispatch(&kernel, &grants, &handles, "REDIS_INCREMENT", json!({ "key": counter }));
        let failure = outcome(&broken, "REDIS_FAILED");
        assert_eq!((failure["kind"].as_str(), failure["code"].as_str()), (Some("redis"), Some("ERR")));
        dispatch(&kernel, &grants, &handles, "REDIS_DELETE", json!({ "key": counter }));
    }
}
