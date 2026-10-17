// H.6 parity: every recorded iconify fixture, replayed through the kernel against `std-iconify.lolo`, makes the
// request the manual provider made (method, URL, compared headers, body, timeout) and emits the recorded outcome.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-iconify.lolo",
    trait_name: "IconifyClient",
    fixtures_dir: "iconify",
    events: &[
        ActionEvents { action: "svgExists", input: "ICONIFY_SVG_EXISTS", result: "ICONIFY_SVG_EXISTS_RESULT", failure: None, data_field: None },
        ActionEvents { action: "search", input: "ICONIFY_SEARCH", result: "ICONIFY_SEARCH_RESULT", failure: None, data_field: None },
        ActionEvents { action: "getIconBody", input: "ICONIFY_GET_ICON_BODY", result: "ICONIFY_ICON_BODY_RESULT", failure: None, data_field: None },
    ],
    env_knobs: &[("ICONIFY_USER_AGENT", "userAgent"), ("ICONIFY_TIMEOUT_MS", "timeoutMs")],
    secret_env: &[],
    compared_headers: &[
        "content-type",
        "accept",
        "authorization",
        "x-api-key",
        "anthropic-version",
        "anthropic-beta",
        "openai-organization",
        "user-agent",
    ],
    default_timeout_ms: 6000,
    base: None,
    ..ParitySpec::DEFAULT
};

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert!(all.len() >= 22, "expected the recorded iconify cases, found {}", all.len());
    let excluded: Vec<String> = all
        .iter()
        .filter(|f| !lolo_parity::is_ok(f))
        .map(|f| lolo_parity::text(f, "case").to_string())
        .collect();
    let excluded: Vec<&str> = excluded.iter().map(String::as_str).collect();
    assert_eq!(lolo_parity::replay_all_except(&SPEC, &excluded), 18, "the cases with a recorded result");
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    let rejected: Vec<String> = lolo_parity::fixtures(&SPEC)
        .iter()
        .filter(|f| !lolo_parity::is_ok(f))
        .map(|f| lolo_parity::text(f, "case").to_string())
        .collect();
    assert_eq!(rejected.len(), 4);
    let rejected: Vec<&str> = rejected.iter().map(String::as_str).collect();
    lolo_parity::assert_validation_only_excluded(&SPEC, &rejected);
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    lolo_parity::drifted_url_control(&SPEC, "search", "limit=3", "limit=4");
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "search", "icons", serde_json::json!(["logos:github-icon"]));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
