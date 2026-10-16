// H.6 parity for storage. The recorded storage fixtures are stateful step scenarios (one file = an ordered list of
// `steps`, each with params, clockMs, exchanges and outcome), not one record per file, so each step is flattened
// into the single-record shape the shared harness replays and run on its own kernel session. The harness lowers
// the atom without an egress override, so the test lowers it here, patches the knobs the recorded env names and
// declares the recorded endpoint in egress, then hands the schema to `ParitySession::with_schema`.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, FixtureAuthority, ParitySession, ParitySpec, ReplayStep};
use serde_json::{json, Value};
use std::path::PathBuf;

const FAILED: Option<&str> = Some("STORAGE_FAILED");

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-storage.lolo",
    trait_name: "StorageClient",
    fixtures_dir: "storage",
    events: &[
        ActionEvents { action: "upload", input: "STORAGE_UPLOAD", result: "STORAGE_UPLOADED", failure: FAILED },
        ActionEvents { action: "uploadMany", input: "STORAGE_UPLOAD_MANY", result: "STORAGE_UPLOADED_MANY", failure: FAILED },
        ActionEvents { action: "download", input: "STORAGE_DOWNLOAD", result: "STORAGE_DOWNLOADED", failure: FAILED },
        ActionEvents { action: "list", input: "STORAGE_LIST", result: "STORAGE_LISTED", failure: FAILED },
        ActionEvents { action: "delete", input: "STORAGE_DELETE", result: "STORAGE_DELETED", failure: FAILED },
        ActionEvents { action: "getSignedUrl", input: "STORAGE_GET_SIGNED_URL", result: "STORAGE_SIGNED_URL", failure: FAILED },
    ],
    env_knobs: &[],
    secret_env: &["STORAGE_ACCESS_KEY_ID", "STORAGE_SECRET_ACCESS_KEY"],
    compared_headers: &["content-type", "authorization"],
    default_timeout_ms: 300_000,
    base: None,
};

const KNOBS: [(&str, &str); 4] = [
    ("STORAGE_ENDPOINT", "endpoint"),
    ("STORAGE_REGION", "region"),
    ("STORAGE_BUCKET", "bucket"),
    ("STORAGE_PUBLIC_URL_BASE", "publicUrlBase"),
];

// Origins the atom derives from the bucket (virtual-hosted and regional path-style AWS) must be declared in egress.
const AWS_ORIGINS: [&str; 4] = [
    "https://media.s3.us-east-1.amazonaws.com",
    "https://media.s3.ap-southeast-2.amazonaws.com",
    "https://s3.ap-southeast-2.amazonaws.com",
    "https://s3.us-east-1.amazonaws.com",
];

// D-A80: the registry's param check rejects these before anything is sent; the typed event payload replaces it.
const VALIDATION_ONLY: [&str; 8] = [
    "memory-validation-failures#0",
    "memory-validation-failures#1",
    "memory-validation-failures#2",
    "memory-validation-failures#3",
    "memory-validation-failures#4",
    "memory-validation-failures#5",
    "memory-validation-failures#6",
    "memory-upload-forms#4",
];

// Memory-backend steps whose outcome is decided before any backend runs (no exchanges, same message on S3).
const MEMORY_BACKEND_FREE: [&str; 8] = [
    "memory-upload-forms#9",
    "memory-upload-forms#11",
    "memory-upload-forms#13",
    "memory-upload-forms#14",
    "memory-upload-forms#15",
    "memory-upload-forms#16",
    "memory-upload-many#3",
    "memory-upload-many#4",
];

// The kernel presigns only whole-second lifetimes of 1..=604800; the provider signed 0 and 1.5 as given.
const PRESIGN_LIFETIME_REFUSED: [&str; 2] = ["s3-signed-urls#6", "s3-signed-urls#7"];

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../orbital-integrations/fixtures/storage")
}

fn steps() -> Vec<Value> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir())
        .expect("storage fixtures")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let file: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
        let name = file["case"].as_str().expect("case").to_string();
        for (index, step) in file["steps"].as_array().expect("steps").iter().enumerate() {
            let env = file["env"].clone();
            out.push(json!({
                "case": format!("{name}#{index}"),
                "action": step["action"],
                "params": step["params"],
                "exchanges": step["exchanges"],
                "outcome": step["outcome"],
                "clockMs": step["clockMs"],
                "env": env,
            }));
        }
    }
    out
}

fn name(step: &Value) -> &str {
    lolo_parity::text(step, "case")
}

fn session(step: &Value) -> ParitySession<'static> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(SPEC.atom);
    let schema = orbital_test_support::lower_lolo_file(&path).unwrap_or_else(|e| panic!("lower: {e}"));
    let mut schema = serde_json::to_value(&schema).expect("schema json");
    for (variable, knob) in KNOBS {
        let Some(raw) = step["env"][variable].as_str() else { continue };
        let mut patched = false;
        for orbital in schema["orbitals"].as_array_mut().expect("orbitals") {
            for t in orbital["traits"].as_array_mut().into_iter().flatten() {
                if let Some(entry) = t["config"].get_mut(knob) {
                    entry["default"] = json!(raw);
                    patched = true;
                }
            }
        }
        assert!(patched, "{}: no knob {knob}", name(step));
        if knob == "endpoint" {
            schema["egress"].as_array_mut().expect("egress").push(json!(raw));
        }
    }
    for origin in AWS_ORIGINS {
        schema["egress"].as_array_mut().expect("egress").push(json!(origin));
    }
    let schema = serde_json::from_value(schema).expect("schema back");
    ParitySession::with_schema(&SPEC, schema, step, FixtureAuthority::default())
}

fn replay(step: &Value) -> lolo_parity::Run {
    let at = step["clockMs"].as_u64().expect("clockMs");
    session(step).run(step, ReplayStep { at_ms: Some(at), ..ReplayStep::default() })
}

fn check(step: &Value) {
    let at = step["clockMs"].as_u64().expect("clockMs");
    session(step).check(step, ReplayStep { at_ms: Some(at), ..ReplayStep::default() });
}

fn listed(names: &[&str]) -> impl Fn(&Value) -> bool + '_ {
    move |step| names.contains(&name(step))
}

fn is_memory(step: &Value) -> bool {
    name(step).starts_with("memory-")
}

fn fully_replayed() -> Vec<Value> {
    steps()
        .into_iter()
        .filter(|step| {
            let in_memory = is_memory(step);
            !(in_memory && !listed(&MEMORY_BACKEND_FREE)(step))
                && !listed(&PRESIGN_LIFETIME_REFUSED)(step)
        })
        .collect()
}

#[test]
fn the_fixture_set_is_the_recorded_94_steps() {
    assert_eq!(steps().len(), 94);
}

#[test]
fn every_recorded_s3_exchange_and_outcome_is_replayed_by_the_atom() {
    let all = fully_replayed();
    assert_eq!(all.len(), 48);
    for step in &all {
        check(step);
    }
}

#[test]
fn the_validation_only_steps_are_exactly_the_registry_param_errors_that_send_nothing() {
    let mut rejected: Vec<String> = steps()
        .iter()
        .filter(|s| s["outcome"]["code"] == "VALIDATION_ERROR")
        .map(|s| name(s).to_string())
        .collect();
    rejected.sort();
    let mut listed: Vec<String> = VALIDATION_ONLY.iter().map(|c| c.to_string()).collect();
    listed.sort();
    assert_eq!(rejected, listed);
    for step in steps().iter().filter(|s| VALIDATION_ONLY.contains(&name(s))) {
        assert!(step["exchanges"].as_array().expect("exchanges").is_empty(), "{}", name(step));
    }
}

#[test]
fn the_memory_backend_steps_are_not_replayed_because_the_atom_has_no_memory_backend() {
    let skipped: Vec<String> = steps()
        .iter()
        .filter(|s| is_memory(s) && !MEMORY_BACKEND_FREE.contains(&name(s)) && !VALIDATION_ONLY.contains(&name(s)))
        .map(|s| name(s).to_string())
        .collect();
    assert_eq!(skipped.len(), 52 - MEMORY_BACKEND_FREE.len() - 8);
    for step in steps().iter().filter(|s| is_memory(s)) {
        assert!(step["exchanges"].as_array().expect("exchanges").is_empty(), "{}", name(step));
    }
}

#[test]
fn a_presign_lifetime_the_kernel_refuses_fails_explicitly() {
    for step in steps().iter().filter(|s| PRESIGN_LIFETIME_REFUSED.contains(&name(s))) {
        let run = replay(step);
        assert!(run.sent.is_empty(), "{}", name(step));
        assert!(run.results.is_empty(), "{}", name(step));
        assert_eq!(run.failures.len(), 1, "{}", name(step));
    }
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut step = steps().into_iter().find(|s| name(s) == "s3-object-round-trip#5").expect("step");
    let url = step["exchanges"][0]["request"]["url"].as_str().expect("url").replace("max-keys=2", "max-keys=3");
    step["exchanges"][0]["request"]["url"] = json!(url);
    check(&step);
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut step = steps().into_iter().find(|s| name(s) == "s3-object-round-trip#8").expect("step");
    step["outcome"]["data"]["deleted"] = json!(false);
    check(&step);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    for step in steps() {
        for (key, value) in step["env"].as_object().expect("env") {
            if ["_KEY", "_TOKEN", "_SECRET", "_ACCESS_KEY"].iter().any(|s| key.ends_with(s)) {
                let text = value.as_str().expect("env text");
                assert!(text.starts_with("test-") || text.is_empty(), "{}: {key}", name(&step));
            }
        }
        for exchange in step["exchanges"].as_array().expect("exchanges") {
            assert_eq!(exchange["request"]["headers"]["authorization"], lolo_parity::REDACTED, "{}", name(&step));
        }
    }
    let source = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(SPEC.atom)).expect("atom");
    assert!(source.contains(": secret = \"STORAGE_ACCESS_KEY_ID\""));
    assert!(!source.contains("test-key"));
}
