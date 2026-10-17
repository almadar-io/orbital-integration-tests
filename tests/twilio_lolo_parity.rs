// H.6 parity: every recorded twilio fixture, replayed through the kernel against `std-twilio.lolo`, makes the
// request the manual provider made (method, URL, compared headers, body, timeout) and emits the recorded outcome.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-twilio.lolo",
    trait_name: "TwilioClient",
    fixtures_dir: "twilio",
    events: &[
        ActionEvents { action: "sendSMS", input: "TWILIO_SEND_SMS", result: "TWILIO_SMS_RESULT", failure: Some("TWILIO_SMS_FAILED"), data_field: None },
        ActionEvents { action: "sendWhatsApp", input: "TWILIO_SEND_WHATSAPP", result: "TWILIO_WHATSAPP_RESULT", failure: Some("TWILIO_WHATSAPP_FAILED"), data_field: None },
    ],
    env_knobs: &[("TWILIO_ACCOUNT_SID", "accountSid"), ("TWILIO_PHONE_NUMBER", "phoneNumber")],
    secret_env: &["TWILIO_ACCOUNT_SID", "TWILIO_AUTH_TOKEN"],
    compared_headers: &["content-type", "accept", "authorization", "x-api-key", "user-agent"],
    default_timeout_ms: 30000,
    base: None,
    ..ParitySpec::DEFAULT
};

const VALIDATION_ONLY: [&str; 4] = [
    "unknown-action",
    "validation-missing-body",
    "validation-unknown-param",
    "validation-wrong-type",
];

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 24, "the recorded twilio cases");
    assert_eq!(lolo_parity::replay_all_except(&SPEC, &VALIDATION_ONLY), 20, "the cases that send or map a reply");
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    lolo_parity::drifted_url_control(&SPEC, "send-sms", "Messages.json", "Message.json");
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "send-sms", "status", serde_json::json!("sent"));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
