// H.6 parity: every recorded ml fixture, replayed through the kernel against `std-ml.lolo`, makes the request the
// manual provider made (method, URL, compared headers, body, timeout) and emits the recorded outcome.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/connect/atoms/std-ml.lolo",
    trait_name: "MlClient",
    fixtures_dir: "ml",
    events: &[ActionEvents {
        action: "infer",
        input: "ML_INFER",
        result: "ML_INFERRED",
        failure: Some("ML_INFER_FAILED"),
        data_field: None,
    }],
    env_knobs: &[("MASAR_ML_TRAIT", "traitName"), ("MASAR_ML_TIMEOUT_MS", "timeoutMs")],
    secret_env: &[],
    compared_headers: &["content-type", "accept"],
    default_timeout_ms: 60000,
    base: Some(("origin", "http://localhost:3000")),
    ..ParitySpec::DEFAULT
};

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 24, "the recorded ml cases");
    let rejected = ["unknown-action", "validation-missing-input", "validation-unknown-param", "validation-wrong-type"];
    // `base` pins the origin knob for every fixture, so an unset MASAR_URL cannot be replayed (reported, not weakened).
    let excluded = [rejected.as_slice(), &["credential-absent-url"]].concat();
    assert_eq!(lolo_parity::replay_all_except(&SPEC, &excluded), 19, "the cases that send a request or map a reply");
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(
        &SPEC,
        &["unknown-action", "validation-missing-input", "validation-unknown-param", "validation-wrong-type"],
    );
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    lolo_parity::drifted_url_control(&SPEC, "infer", "/events", "/event");
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "infer", "confidence", serde_json::json!(0.1));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
