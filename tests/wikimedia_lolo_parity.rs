// H.6 parity: every recorded wikimedia fixture, replayed through the kernel against `std-wikimedia.lolo`, makes the
// request the manual provider made (method, URL, compared headers, body, timeout) and emits the recorded outcome.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/connect/atoms/std-wikimedia.lolo",
    trait_name: "WikimediaClient",
    fixtures_dir: "wikimedia",
    events: &[ActionEvents {
        action: "getPage",
        input: "WIKIMEDIA_GET_PAGE",
        result: "WIKIMEDIA_PAGE_RESULT",
        failure: Some("WIKIMEDIA_PAGE_FAILURE"),
        data_field: None,
    }],
    env_knobs: &[("WIKIMEDIA_USER_AGENT", "userAgent"), ("WIKIMEDIA_TIMEOUT_MS", "timeoutMs")],
    secret_env: &[],
    compared_headers: &["content-type", "accept", "authorization", "x-api-key", "user-agent"],
    default_timeout_ms: 6000,
    base: None,
    ..ParitySpec::DEFAULT
};

const VALIDATION_ONLY: [&str; 4] =
    ["unknown-action", "validation-missing-title", "validation-unknown-param", "validation-wrong-type"];

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 16, "the recorded wikimedia cases");
    assert_eq!(lolo_parity::replay_all_except(&SPEC, &VALIDATION_ONLY), 12, "the cases with a recorded result");
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    lolo_parity::drifted_url_control(&SPEC, "get-page", "pithumbsize=240", "pithumbsize=241");
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "get-page", "title", serde_json::json!("Grace Hopper"));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
