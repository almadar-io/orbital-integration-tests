// H.6 parity: every recorded banking fixture that sends a request or maps a reply, replayed through the kernel
// against `std-banking.lolo`, makes the request the manual provider made and emits the recorded outcome.
// The access-token login keeps its own 15 s deadline; resource requests keep 20 s.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use std::time::Duration;

use lolo_parity::{
    ActionEvents, Deadline, ExchangeContract, FixtureAuthority, FixtureCredential, ParitySession, ParitySpec,
    ReplayStep,
};
use orbital_core::runtime::http_request::CredentialSlot;
use serde_json::{json, Value as Json};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-banking.lolo",
    trait_name: "BankingClient",
    fixtures_dir: "banking",
    events: &[
        ActionEvents { action: "createRequisition", input: "BANKING_CREATE_REQUISITION", result: "BANKING_REQUISITION_RESULT", failure: Some("BANKING_FAILED"), data_field: None },
        ActionEvents { action: "listAccounts", input: "BANKING_LIST_ACCOUNTS", result: "BANKING_ACCOUNTS_RESULT", failure: Some("BANKING_FAILED"), data_field: None },
        ActionEvents { action: "listTransactions", input: "BANKING_LIST_TRANSACTIONS", result: "BANKING_TRANSACTIONS_RESULT", failure: Some("BANKING_FAILED"), data_field: None },
    ],
    env_knobs: &[],
    secret_env: &["GOCARDLESS_SECRET_ID", "GOCARDLESS_SECRET_KEY"],
    compared_headers: &["content-type", "accept", "authorization", "x-api-key", "user-agent"],
    default_timeout_ms: 20000,
    base: None,
};

const VALIDATION_ONLY: [&str; 4] =
    ["unknown-action", "validation-missing-institution", "validation-unknown-param", "validation-wrong-type"];

// Approved divergence: the provider cut the error text at 300 characters; the atom passes the full text.
const CAPPED_ERROR: &str = "create-requisition-error-body-over-300-chars";
// The driver reports a rejected login with its grant phase and status, which the atom maps to the recorded text.
const LOGIN_REJECTED: &str = "token-rejected";

fn authority() -> FixtureAuthority {
    FixtureAuthority {
        credentials: vec![FixtureCredential {
            slot: CredentialSlot::from("gocardless:access"),
            principal: None,
            value: None,
            ttl_ms: None,
        }],
        now_ms: None,
    }
}

fn contracts(fixture: &Json) -> Vec<ExchangeContract> {
    let count = fixture["exchanges"].as_array().expect("exchanges").len();
    (0..count)
        .map(|index| ExchangeContract {
            timeout: if index == 0 { Deadline::Exact(Some(Duration::from_millis(15000))) } else { Deadline::Default },
            ..Default::default()
        })
        .collect()
}

fn replay(fixture: &Json) {
    let mut contracts = contracts(fixture);
    if fixture["exchanges"].as_array().expect("exchanges").first().is_some_and(|e| {
        !e["request"]["url"].as_str().expect("url").ends_with("/token/new/")
    }) {
        contracts[0] = ExchangeContract::default();
    }
    ParitySession::new(&SPEC, fixture, authority())
        .check(fixture, ReplayStep { exchanges: contracts, ..Default::default() });
}

fn replay_all_except(excluded: &[&str]) -> usize {
    let mut replayed = 0;
    for fixture in lolo_parity::fixtures(&SPEC) {
        if excluded.contains(&lolo_parity::text(&fixture, "case")) {
            continue;
        }
        replay(&fixture);
        replayed += 1;
    }
    replayed
}

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 20, "the recorded banking cases");
    let mut excluded = VALIDATION_ONLY.to_vec();
    excluded.push(CAPPED_ERROR);
    assert_eq!(replay_all_except(&excluded), 15, "the cases that send or map a reply");
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
fn an_error_body_over_300_characters_is_reported_in_full() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, CAPPED_ERROR);
    let body = fixture["exchanges"][1]["response"]["body"].as_str().expect("body").to_string();
    assert!(body.len() > 300);
    fixture["outcome"]["message"] = json!(format!("GoCardless error 400: {body}"));
    replay(&fixture);
}

#[test]
#[should_panic(expected = "emitted failure")]
fn the_recorded_capped_error_text_is_not_what_the_atom_emits() {
    replay(&lolo_parity::fixture_named(&SPEC, CAPPED_ERROR));
}

#[test]
fn the_rejected_login_fixture_is_replayed_by_the_atom() {
    replay(&lolo_parity::fixture_named(&SPEC, LOGIN_REJECTED));
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "list-accounts");
    let url = fixture["exchanges"][1]["request"]["url"].as_str().expect("url").replace("requisitions", "requisition");
    fixture["exchanges"][1]["request"]["url"] = json!(url);
    replay(&fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "create-requisition");
    fixture["outcome"]["data"]["link"] = json!("https://elsewhere.example/");
    replay(&fixture);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
