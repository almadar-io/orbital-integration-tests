// H.6 parity: every recorded youtube fixture, replayed through the kernel against `std-youtube.lolo`, makes the
// request the manual provider made (method, URL, compared headers, body, timeout) and emits the recorded outcome.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-youtube.lolo",
    trait_name: "YoutubeClient",
    fixtures_dir: "youtube",
    events: &[
        ActionEvents { action: "search", input: "YOUTUBE_SEARCH", result: "YOUTUBE_SEARCH_RESULT", failure: Some("YOUTUBE_SEARCH_FAILED") },
        ActionEvents { action: "getVideo", input: "YOUTUBE_GET_VIDEO", result: "YOUTUBE_VIDEO_RESULT", failure: Some("YOUTUBE_VIDEO_FAILED") },
        ActionEvents { action: "getChannel", input: "YOUTUBE_GET_CHANNEL", result: "YOUTUBE_CHANNEL_RESULT", failure: Some("YOUTUBE_CHANNEL_FAILED") },
    ],
    env_knobs: &[],
    secret_env: &["YOUTUBE_API_KEY"],
    compared_headers: &["content-type", "accept", "authorization", "x-api-key"],
    default_timeout_ms: 30000,
    base: None,
};

const VALIDATION_ONLY: [&str; 4] = [
    "unknown-action",
    "validation-missing-query",
    "validation-unknown-param",
    "validation-wrong-type",
];

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert!(all.len() >= 31, "expected the recorded youtube cases, found {}", all.len());
    assert_eq!(lolo_parity::replay_all_except(&SPEC, &VALIDATION_ONLY), 27, "the cases that send a request or map a reply");
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    lolo_parity::drifted_url_control(&SPEC, "get-video", "id=vid1", "id=vid9");
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "get-video", "title", serde_json::json!("Other"));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}

const DBG: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/zz-youtube-dbg.lolo", trait_name: "YoutubeClient", fixtures_dir: "youtube",
    events: &[
        ActionEvents { action: "search", input: "YOUTUBE_SEARCH", result: "YOUTUBE_REQUEST", failure: Some("YOUTUBE_REJECTED") },
        ActionEvents { action: "getVideo", input: "YOUTUBE_GET_VIDEO", result: "YOUTUBE_REQUEST", failure: Some("YOUTUBE_REJECTED") },
        ActionEvents { action: "getChannel", input: "YOUTUBE_GET_CHANNEL", result: "YOUTUBE_REQUEST", failure: Some("YOUTUBE_REJECTED") },
    ],
    env_knobs: &[], secret_env: &["YOUTUBE_API_KEY"], compared_headers: &["accept"], default_timeout_ms: 30000, base: None,
};

#[test]
fn zz_debug_each_case() {
    for case in ["search-error-with-several-reasons", "search-client-error-with-a-text-body"] {
        let f = lolo_parity::fixture_named(&DBG, case);
        let r = lolo_parity::run(&DBG, &f);
        eprintln!("DBG {case}: requests={:?} rejected={:?} sent={}", r.results, r.failures, r.sent.len());
    }
}
