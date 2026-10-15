// H.6 parity: the arxiv fixtures replayed through the kernel against `std-arxiv.lolo`. The atom carries the request side
// and the empty-result answers; a 2xx feed needs an Atom reader no std operator provides, so those cases are listed.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-arxiv.lolo",
    trait_name: "ArxivClient",
    fixtures_dir: "arxiv",
    events: &[ActionEvents { action: "search", input: "ARXIV_SEARCH", result: "ARXIV_SEARCH_RESULT", failure: None }],
    env_knobs: &[("ARXIV_TIMEOUT_MS", "timeoutMs")],
    secret_env: &[],
    compared_headers: &["accept"],
    default_timeout_ms: 10000,
    base: None,
};

const NEEDS_ATOM_READER: [&str; 3] = ["search", "search-empty-feed", "search-max-results-and-reserved-characters"];
const VALIDATION_ONLY: [&str; 4] = [
    "unknown-action",
    "validation-missing-query",
    "validation-unknown-param",
    "validation-wrong-type",
];

#[test]
fn every_replayable_outcome_is_replayed_by_the_atom() {
    let excluded: Vec<&str> = NEEDS_ATOM_READER.iter().chain(VALIDATION_ONLY.iter()).copied().collect();
    assert_eq!(lolo_parity::replay_all_except(&SPEC, &excluded), 3, "blank query, http 400, http 503");
}

#[test]
fn the_validation_cases_are_registry_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
fn the_2xx_cases_still_make_the_recorded_request_and_hand_the_feed_on() {
    for case in NEEDS_ATOM_READER {
        let fixture = lolo_parity::fixture_named(&SPEC, case);
        let run = lolo_parity::run(&SPEC, &fixture);
        assert_eq!(run.unused, 0, "{case}: recorded request not sent");
        assert_eq!(run.sent.len(), 1, "{case}: request count");
        assert!(run.results.is_empty(), "{case}: the atom answers the feed, not parsed results");
    }
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    lolo_parity::drifted_url_control(&SPEC, "search-http-503-yields-no-results", "max_results=8", "max_results=9");
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "search-http-503-yields-no-results", "results", serde_json::json!(["x"]));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
