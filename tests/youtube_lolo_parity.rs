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
        ActionEvents { action: "search", input: "YOUTUBE_SEARCH", result: "YOUTUBE_SEARCH_RESULT", failure: Some("YOUTUBE_SEARCH_FAILED"), data_field: Some("items") },
        ActionEvents { action: "getVideo", input: "YOUTUBE_GET_VIDEO", result: "YOUTUBE_VIDEO_RESULT", failure: Some("YOUTUBE_VIDEO_FAILED"), data_field: None },
        ActionEvents { action: "getChannel", input: "YOUTUBE_GET_CHANNEL", result: "YOUTUBE_CHANNEL_RESULT", failure: Some("YOUTUBE_CHANNEL_FAILED"), data_field: None },
    ],
    env_knobs: &[],
    secret_env: &["YOUTUBE_API_KEY"],
    compared_headers: &["content-type", "accept", "authorization", "x-api-key"],
    default_timeout_ms: 30000,
    base: None,
    ..ParitySpec::DEFAULT
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
    let mut excluded = VALIDATION_ONLY.to_vec();
    excluded.extend([NO_ITEMS, UNREADABLE]);
    assert_eq!(lolo_parity::replay_all_except(&SPEC, &excluded), 25, "the cases that send a request or map a reply");
}

/// Owner-approved divergence (2026-10-16): a search reply without `items` gives `[]`, where the provider gave null.
const NO_ITEMS: &str = "search-no-items-field";

/// D15 (owner 2026-10-15): an unreadable 2xx reply fails explicitly, where the provider gave null.
const UNREADABLE: &str = "search-unreadable-ok-reply";

#[test]
fn an_unreadable_ok_reply_fails_explicitly_d15() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, UNREADABLE);
    assert_eq!(fixture["outcome"], serde_json::json!({"ok": true, "data": null}), "the provider's recorded fallback");
    fixture["outcome"] = serde_json::json!({
        "ok": false,
        "code": "MALFORMED_REPLY",
        "message": "YouTube API replied with a body that is not valid JSON"
    });
    lolo_parity::check(&SPEC, &fixture);
}

#[test]
fn a_search_reply_without_items_is_an_empty_list_an_approved_divergence() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, NO_ITEMS);
    assert_eq!(fixture["outcome"]["data"], serde_json::Value::Null, "the provider's recorded null");
    fixture["outcome"]["data"] = serde_json::json!([]);
    lolo_parity::check(&SPEC, &fixture);
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
