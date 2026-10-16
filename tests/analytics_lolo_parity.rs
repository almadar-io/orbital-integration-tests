// H.6 parity: every recorded analytics fixture replayed through the kernel against `std-analytics.lolo`
// makes the delivery the provider made (POST, text/plain, the recorded body bytes) and emits the recorded outcome.
// Browser facts come from the fixture's `page` (D-A87 `browser/page-facts`); delivery is `browser/beacon`.
// The collector is the atom's `endpoint` url knob (a beacon URL is a literal or a url knob), so a fixture's `endpoint`
// param becomes that knob's value and the atom's internal events carry no endpoint.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, Deadline, ExchangeContract, ParitySession, ParitySpec, ReplayStep, FixtureAuthority};
use serde_json::{json, Value as Json};

const ORIGIN: &str = "https://almadar-analytics--kflow-b3a39.europe-west4.hosted.app";

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-analytics.lolo",
    trait_name: "Analytics",
    fixtures_dir: "analytics",
    events: &[
        ActionEvents { action: "pageview", input: "ANALYTICS_PAGEVIEW", result: "REPORTED", failure: Some("ANALYTICS_FAILED") },
        ActionEvents { action: "track", input: "ANALYTICS_TRACK", result: "REPORTED", failure: Some("ANALYTICS_FAILED") },
    ],
    env_knobs: &[("ANALYTICS_COLLECTOR", "endpoint")],
    secret_env: &[],
    compared_headers: &["content-type", "accept"],
    default_timeout_ms: 0,
    base: None,
};

fn resolve_base(value: &mut Json) {
    match value {
        Json::String(text) => *text = text.replace("{base}", ORIGIN),
        Json::Array(items) => items.iter_mut().for_each(resolve_base),
        Json::Object(map) => map.values_mut().for_each(resolve_base),
        _ => {}
    }
}

fn adapted(mut fixture: Json) -> Json {
    resolve_base(&mut fixture);
    let endpoint = fixture["params"].as_object_mut().expect("params").remove("endpoint").expect("endpoint param");
    fixture["env"]["ANALYTICS_COLLECTOR"] = endpoint;
    fixture
}

fn adapted_fixtures() -> Vec<Json> {
    lolo_parity::fixtures(&SPEC).into_iter().map(adapted).collect()
}

fn replay(fixture: &Json) {
    let count = fixture["exchanges"].as_array().expect("exchanges").len();
    let exchanges = (0..count)
        .map(|_| ExchangeContract { timeout: Deadline::Exact(None), ..Default::default() })
        .collect();
    ParitySession::new(&SPEC, fixture, FixtureAuthority::default())
        .check(fixture, ReplayStep { exchanges, ..Default::default() });
}

fn named(case: &str) -> Json {
    adapted_fixtures().into_iter().find(|f| lolo_parity::text(f, "case") == case).unwrap_or_else(|| panic!("fixture {case}"))
}

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = adapted_fixtures();
    assert_eq!(all.len(), 23, "the recorded analytics cases");
    for fixture in &all {
        replay(fixture);
    }
}

#[test]
fn the_cases_that_send_nothing_are_replayed_with_zero_deliveries() {
    let silent: Vec<Json> = adapted_fixtures()
        .into_iter()
        .filter(|f| f["exchanges"].as_array().expect("exchanges").is_empty())
        .collect();
    assert_eq!(silent.len(), 14, "empty collectors, local hosts and the three validation errors");
}

#[test]
fn a_public_host_that_merely_contains_localhost_still_sends() {
    let fixture = named("pageview-from-a-public-host-that-contains-localhost");
    assert_eq!(fixture["exchanges"].as_array().expect("exchanges").len(), 1);
    replay(&fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = named("pageview");
    fixture["outcome"]["data"] = json!({ "sent": false });
    replay(&fixture);
}

#[test]
#[should_panic(expected = "request body bytes")]
fn a_drifted_recorded_body_fails_the_replay() {
    let mut fixture = named("pageview");
    let body = fixture["exchanges"][0]["request"]["body"].as_str().expect("body").replace("1440", "1441");
    fixture["exchanges"][0]["request"]["body"] = json!(body);
    replay(&fixture);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
