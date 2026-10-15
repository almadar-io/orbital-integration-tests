// H.6 parity for `calendar`: every recorded fixture that sends a request is replayed through the kernel against
// `std-google-calendar.lolo`. The recorded service-account JWT exchange (RS256 assertion at oauth2/v4/token) has no
// counterpart in the closed `auth` set, so the atom takes a bearer access token and each replay drops that first
// exchange and supplies the token as a test secret; everything after it is compared unchanged.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};
use serde_json::{json, Value};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-google-calendar.lolo",
    trait_name: "CalendarClient",
    fixtures_dir: "calendar",
    events: &[
        ActionEvents { action: "listEvents", input: "CALENDAR_LIST_EVENTS", result: "CALENDAR_EVENTS_LISTED", failure: Some("CALENDAR_LIST_EVENTS_FAILED") },
        ActionEvents { action: "createEvent", input: "CALENDAR_CREATE_EVENT", result: "CALENDAR_EVENT_CREATED", failure: Some("CALENDAR_CREATE_EVENT_FAILED") },
        ActionEvents { action: "updateEvent", input: "CALENDAR_UPDATE_EVENT", result: "CALENDAR_EVENT_UPDATED", failure: Some("CALENDAR_UPDATE_EVENT_FAILED") },
        ActionEvents { action: "deleteEvent", input: "CALENDAR_DELETE_EVENT", result: "CALENDAR_EVENT_DELETED", failure: Some("CALENDAR_DELETE_EVENT_FAILED") },
        ActionEvents { action: "freeBusy", input: "CALENDAR_FREE_BUSY", result: "CALENDAR_FREE_BUSY_RESULT", failure: Some("CALENDAR_FREE_BUSY_FAILED") },
        ActionEvents { action: "watch", input: "CALENDAR_WATCH", result: "CALENDAR_WATCHED", failure: Some("CALENDAR_WATCH_FAILED") },
        ActionEvents { action: "stopWatch", input: "CALENDAR_STOP_WATCH", result: "CALENDAR_WATCH_STOPPED", failure: Some("CALENDAR_STOP_WATCH_FAILED") },
    ],
    env_knobs: &[("GOOGLE_CALENDAR_ID", "calendarId")],
    secret_env: &["GOOGLE_CALENDAR_ACCESS_TOKEN"],
    compared_headers: &["content-type", "accept", "authorization"],
    default_timeout_ms: 30000,
    base: None,
};

const VALIDATION_ONLY: [&str; 4] =
    ["unknown-action", "validation-missing-summary", "validation-unknown-param", "validation-wrong-type"];
/// Registry host behavior with no atom counterpart: a provider that is not configured never starts.
const NOT_CONFIGURED: [&str; 2] = ["not-configured-calendar-id-alone", "not-configured-without-credentials"];
/// The JWT exchange itself fails: nothing of it exists in the atom.
const TOKEN_EXCHANGE: [&str; 1] = ["token-rejected"];
/// The timed end is `start + durationMinutes`, which needs a time-of-day ISO parse `time/parse` does not do.
const TIMED_END: [&str; 1] = ["create-event-duration"];

const TEST_TOKEN: &str = "test-access-token";

fn is_token_exchange(exchange: &Value) -> bool {
    exchange["request"]["url"].as_str() == Some("https://www.googleapis.com/oauth2/v4/token")
}

fn prepared(mut fixture: Value) -> Value {
    let exchanges = fixture["exchanges"].as_array().expect("exchanges").clone();
    let kept: Vec<Value> = match exchanges.first() {
        Some(first) if is_token_exchange(first) => exchanges[1..].to_vec(),
        _ => exchanges,
    };
    fixture["exchanges"] = Value::Array(kept);
    fixture["env"]["GOOGLE_CALENDAR_ACCESS_TOKEN"] = json!(TEST_TOKEN);
    fixture
}

fn excluded() -> Vec<&'static str> {
    [&VALIDATION_ONLY[..], &NOT_CONFIGURED[..], &TOKEN_EXCHANGE[..], &TIMED_END[..]].concat()
}

fn replayable() -> Vec<Value> {
    lolo_parity::fixtures(&SPEC)
        .into_iter()
        .filter(|f| !excluded().contains(&lolo_parity::text(f, "case")))
        .map(prepared)
        .collect()
}

#[test]
fn every_recorded_request_and_outcome_after_the_token_exchange_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 45, "the recorded calendar cases");
    let cases = replayable();
    assert_eq!(cases.len(), 45 - excluded().len());
    let mut failed = Vec::new();
    let only = std::env::var("CALENDAR_CASE").ok();
    for fixture in &cases {
        if only.as_deref().is_some_and(|c| c != lolo_parity::text(fixture, "case")) {
            continue;
        }
        let outcome = std::panic::catch_unwind(|| lolo_parity::check(&SPEC, fixture));
        if let Err(cause) = outcome {
            let detail = cause.downcast_ref::<String>().cloned().unwrap_or_default();
            failed.push(detail);
        }
    }
    assert!(failed.is_empty(), "{} of {} cases failed:\n{}", failed.len(), cases.len(), failed.join("\n---\n"));
}

#[test]
fn the_validation_only_cases_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
fn the_cases_the_atom_cannot_express_are_exactly_the_listed_ones() {
    let by_case = |case: &str| lolo_parity::fixture_named(&SPEC, case);
    for case in NOT_CONFIGURED {
        let fixture = by_case(case);
        assert_eq!(fixture["outcome"]["code"], "NOT_CONFIGURED");
        assert!(fixture["exchanges"].as_array().expect("exchanges").is_empty());
    }
    let rejected = by_case("token-rejected");
    assert!(is_token_exchange(&rejected["exchanges"][0]));
    assert_eq!(rejected["exchanges"].as_array().expect("exchanges").len(), 1);
    let timed = by_case("create-event-duration");
    assert!(timed["params"]["end"].is_null() && timed["params"]["durationMinutes"].as_f64().is_some_and(|d| d > 0.0));
    for case in NOT_CONFIGURED.iter().chain(&TOKEN_EXCHANGE).chain(&TIMED_END) {
        assert!(!VALIDATION_ONLY.contains(case));
    }
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = prepared(lolo_parity::fixture_named(&SPEC, "list-events-window"));
    let url = lolo_parity::text(&fixture["exchanges"][0]["request"], "url").replace("maxResults=250", "maxResults=251");
    fixture["exchanges"][0]["request"]["url"] = json!(url);
    lolo_parity::check(&SPEC, &fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = prepared(lolo_parity::fixture_named(&SPEC, "list-events-window"));
    fixture["outcome"]["data"]["nextSyncToken"] = json!("sync-other");
    lolo_parity::check(&SPEC, &fixture);
}

#[test]
fn the_atom_holds_no_key_material_and_names_its_credential_only_by_env_name() {
    let source = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(SPEC.atom),
    )
    .expect("read the atom");
    assert!(!source.contains("PRIVATE KEY") && !source.contains("private_key"));
    assert!(source.contains("accessToken : secret = \"GOOGLE_CALENDAR_ACCESS_TOKEN\""));
    assert_eq!(source.matches(": secret").count(), 1);
    for fixture in lolo_parity::fixtures(&SPEC) {
        for exchange in fixture["exchanges"].as_array().expect("exchanges") {
            if let Some(value) = exchange["request"]["headers"].get("authorization") {
                assert_eq!(value, lolo_parity::REDACTED);
            }
        }
    }
}
