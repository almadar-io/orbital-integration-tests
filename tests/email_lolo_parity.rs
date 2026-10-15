// H.6 parity: every recorded email fixture that sends a request or maps a reply, replayed through the kernel
// against `std-email.lolo`. The host picked the vendor by which key was set; the atom's `vendor` knob is that
// choice made explicit, so the replay derives it from the fixture env the same way (SendGrid wins).

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};
use serde_json::{json, Value};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-email.lolo",
    trait_name: "EmailSender",
    fixtures_dir: "email",
    events: &[ActionEvents { action: "send", input: "EMAIL_SEND", result: "EMAIL_SENT", failure: Some("EMAIL_SEND_FAILED") }],
    env_knobs: &[("EMAIL_VENDOR", "vendor"), ("FROM_EMAIL", "fromEmail")],
    secret_env: &["SENDGRID_API_KEY", "RESEND_API_KEY"],
    compared_headers: &["content-type", "accept", "authorization"],
    default_timeout_ms: 30000,
    base: None,
};

const VALIDATION_ONLY: [&str; 4] =
    ["unknown-action", "validation-missing-subject", "validation-unknown-param", "validation-wrong-type"];
const NOT_CONFIGURED: [&str; 2] = ["not-configured-from-email-alone", "not-configured-without-credentials"];

fn with_vendor(mut fixture: Value) -> Value {
    let vendor = if fixture["env"].get("SENDGRID_API_KEY").is_none() && fixture["env"].get("RESEND_API_KEY").is_some() {
        "resend"
    } else {
        "sendgrid"
    };
    fixture["env"]["EMAIL_VENDOR"] = json!(vendor);
    fixture
}

fn excluded() -> Vec<&'static str> {
    VALIDATION_ONLY.iter().chain(NOT_CONFIGURED.iter()).copied().collect()
}

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 26, "the recorded email cases");
    let mut replayed = 0;
    for fixture in all {
        if excluded().contains(&lolo_parity::text(&fixture, "case")) {
            continue;
        }
        lolo_parity::check(&SPEC, &with_vendor(fixture));
        replayed += 1;
    }
    assert_eq!(replayed, 20);
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
fn the_not_configured_cases_send_nothing() {
    for fixture in lolo_parity::fixtures(&SPEC).iter().filter(|f| NOT_CONFIGURED.contains(&lolo_parity::text(f, "case"))) {
        assert_eq!(fixture["outcome"]["code"], "NOT_CONFIGURED");
        assert!(fixture["exchanges"].as_array().expect("exchanges").is_empty());
    }
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "sendgrid-send");
    fixture["exchanges"][0]["request"]["url"] = json!("https://api.sendgrid.com/v3/mail/sent");
    lolo_parity::check(&SPEC, &with_vendor(fixture));
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "resend-send");
    fixture["outcome"]["data"]["id"] = json!("re-msg-other");
    lolo_parity::check(&SPEC, &with_vendor(fixture));
}

#[test]
#[should_panic(expected = "request body")]
fn a_drifted_recorded_body_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "sendgrid-send-named-addresses");
    fixture["exchanges"][0]["request"]["body"] = json!("{}");
    lolo_parity::check(&SPEC, &with_vendor(fixture));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
