// H.6 parity for `calendar`: every recorded fixture that sends a request is replayed through the kernel against
// `std-google-calendar.lolo`, including the service-account JWT exchange (declared `jwt-bearer` auth, assertion
// signed at the recorded dispatch time) and the NOT_CONFIGURED cases (`unconfigured` failure kind).

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{
    ActionEvents, Deadline, ExchangeContract, FixtureAuthority, ParitySession, ParitySpec, ReplayStep,
};
use serde_json::{json, Value};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-google-calendar.lolo",
    trait_name: "CalendarClient",
    fixtures_dir: "calendar",
    events: &[
        ActionEvents { action: "listEvents", input: "CALENDAR_LIST_EVENTS", result: "CALENDAR_EVENTS_LISTED", failure: Some("CALENDAR_LIST_EVENTS_FAILED"), data_field: None },
        ActionEvents { action: "createEvent", input: "CALENDAR_CREATE_EVENT", result: "CALENDAR_EVENT_CREATED", failure: Some("CALENDAR_CREATE_EVENT_FAILED"), data_field: None },
        ActionEvents { action: "updateEvent", input: "CALENDAR_UPDATE_EVENT", result: "CALENDAR_EVENT_UPDATED", failure: Some("CALENDAR_UPDATE_EVENT_FAILED"), data_field: None },
        ActionEvents { action: "deleteEvent", input: "CALENDAR_DELETE_EVENT", result: "CALENDAR_EVENT_DELETED", failure: Some("CALENDAR_DELETE_EVENT_FAILED"), data_field: None },
        ActionEvents { action: "freeBusy", input: "CALENDAR_FREE_BUSY", result: "CALENDAR_FREE_BUSY_RESULT", failure: Some("CALENDAR_FREE_BUSY_FAILED"), data_field: None },
        ActionEvents { action: "watch", input: "CALENDAR_WATCH", result: "CALENDAR_WATCHED", failure: Some("CALENDAR_WATCH_FAILED"), data_field: None },
        ActionEvents { action: "stopWatch", input: "CALENDAR_STOP_WATCH", result: "CALENDAR_WATCH_STOPPED", failure: Some("CALENDAR_STOP_WATCH_FAILED"), data_field: None },
    ],
    env_knobs: &[("GOOGLE_CALENDAR_ID", "calendarId"), ("GOOGLE_CALENDAR_SUBJECT", "subject")],
    secret_env: &["GOOGLE_CALENDAR_SA_KEY", "GOOGLE_CALENDAR_ACCESS_TOKEN"],
    compared_headers: &["content-type", "accept", "authorization"],
    default_timeout_ms: 30000,
    base: None,
    ..ParitySpec::DEFAULT
};

const VALIDATION_ONLY: [&str; 4] =
    ["unknown-action", "validation-missing-summary", "validation-unknown-param", "validation-wrong-type"];
/// D15 (owner): a 2xx reply whose JSON body is unreadable fails explicitly instead of the provider's empty result.
const MALFORMED_REPLY: [&str; 1] = ["list-events-non-json-reply"];

const TOKEN_URL: &str = "https://www.googleapis.com/oauth2/v4/token";
const RECORDED_NOW_MS: u64 = 1_760_000_000_000;

fn is_token_exchange(exchange: &Value) -> bool {
    exchange["request"]["url"].as_str() == Some(TOKEN_URL)
}

fn step(fixture: &Value) -> ReplayStep {
    let exchanges = fixture["exchanges"]
        .as_array()
        .expect("exchanges")
        .iter()
        .map(|exchange| ExchangeContract {
            timeout: if is_token_exchange(exchange) { Deadline::Exact(None) } else { Deadline::Default },
            ..ExchangeContract::default()
        })
        .collect();
    ReplayStep { exchanges, ..ReplayStep::default() }
}

fn session(fixture: &Value) -> ParitySession<'static> {
    ParitySession::new(
        &SPEC,
        fixture,
        FixtureAuthority { now_ms: Some(RECORDED_NOW_MS), ..FixtureAuthority::default() },
    )
}

fn replay(fixture: &Value) {
    session(fixture).check(fixture, step(fixture));
}

fn replayable() -> Vec<Value> {
    lolo_parity::fixtures(&SPEC)
        .into_iter()
        .filter(|f| {
            let case = lolo_parity::text(f, "case");
            !VALIDATION_ONLY.contains(&case) && !MALFORMED_REPLY.contains(&case)
        })
        .collect()
}

#[test]
fn every_recorded_request_and_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 45, "the recorded calendar cases");
    let cases = replayable();
    assert_eq!(cases.len(), 45 - VALIDATION_ONLY.len() - MALFORMED_REPLY.len());
    let mut replayed = Vec::new();
    for fixture in &cases {
        replay(fixture);
        replayed.push(lolo_parity::text(fixture, "case").to_string());
    }
    let expected: Vec<String> = cases.iter().map(|f| lolo_parity::text(f, "case").to_string()).collect();
    assert_eq!(replayed, expected, "replayed fixture identities");
}

#[test]
fn the_validation_only_cases_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
fn an_unreadable_success_body_fails_explicitly_while_an_empty_acknowledgement_still_succeeds() {
    let fixture = lolo_parity::fixture_named(&SPEC, "list-events-non-json-reply");
    let run = session(&fixture).run(&fixture, step(&fixture));
    assert!(run.results.is_empty(), "no result event for an unreadable body");
    assert_eq!(
        run.failures,
        vec![json!({"code": "MALFORMED_REPLY", "message": "Google Calendar replied with a body that is not valid JSON"})]
    );
    assert_eq!(run.sent.len(), 2);
    replay(&lolo_parity::fixture_named(&SPEC, "delete-event"));
}

#[test]
fn the_not_configured_cases_send_nothing_and_name_the_missing_service_account() {
    for case in ["not-configured-calendar-id-alone", "not-configured-without-credentials"] {
        let fixture = lolo_parity::fixture_named(&SPEC, case);
        assert_eq!(fixture["outcome"]["code"], "NOT_CONFIGURED");
        assert!(fixture["exchanges"].as_array().expect("exchanges").is_empty());
    }
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "list-events-window");
    let url = lolo_parity::text(&fixture["exchanges"][1]["request"], "url").replace("maxResults=250", "maxResults=251");
    fixture["exchanges"][1]["request"]["url"] = json!(url);
    replay(&fixture);
}

#[test]
#[should_panic(expected = "request body bytes")]
fn a_drifted_recorded_assertion_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "list-events-window");
    let body = lolo_parity::text(&fixture["exchanges"][0]["request"], "body").replace("assertion=eyJ", "assertion=eyK");
    fixture["exchanges"][0]["request"]["body"] = json!(body);
    replay(&fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "list-events-window");
    fixture["outcome"]["data"]["nextSyncToken"] = json!("sync-other");
    replay(&fixture);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture_beyond_the_recorded_test_service_account() {
    let recorded: Vec<String> = lolo_parity::fixtures(&SPEC)
        .iter()
        .filter_map(|f| f["env"]["GOOGLE_CALENDAR_SA_KEY"].as_str().map(str::to_string))
        .collect();
    let allowed: Vec<&str> = recorded.iter().map(String::as_str).collect();
    lolo_parity::assert_no_secrets_allowing(&SPEC, &allowed);
}
