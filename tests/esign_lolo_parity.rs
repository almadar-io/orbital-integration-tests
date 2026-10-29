// H.6 parity: every recorded esign fixture that sends a request, replayed through the kernel against
// `std-esign.lolo`, makes the request the manual provider made and emits the recorded outcome. The recorded
// `DOCUSIGN_BASE_URL` (origin + account path) is split onto the atom's `origin` (the harness `base`) and `basePath` knobs.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, Deadline, ExchangeContract, FixtureAuthority, ParitySession, ParitySpec, RecordedBody, ReplayStep};
use serde_json::{json, Value};

const ORIGIN: &str = "https://demo.docusign.net";

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/connect/atoms/std-esign.lolo",
    trait_name: "EsignClient",
    fixtures_dir: "esign",
    events: &[
        ActionEvents { action: "sendEnvelope", input: "ESIGN_SEND_ENVELOPE", result: "ESIGN_ENVELOPE_SENT", failure: Some("ESIGN_SEND_ENVELOPE_FAILED"), data_field: None },
        ActionEvents { action: "getEnvelopeStatus", input: "ESIGN_GET_ENVELOPE_STATUS", result: "ESIGN_ENVELOPE_STATUS", failure: Some("ESIGN_GET_ENVELOPE_STATUS_FAILED"), data_field: None },
        ActionEvents { action: "downloadDocument", input: "ESIGN_DOWNLOAD_DOCUMENT", result: "ESIGN_DOCUMENT_DOWNLOADED", failure: Some("ESIGN_DOWNLOAD_DOCUMENT_FAILED"), data_field: None },
    ],
    env_knobs: &[("DOCUSIGN_BASE_PATH", "basePath")],
    secret_env: &["DOCUSIGN_ACCESS_TOKEN"],
    compared_headers: &["content-type", "accept", "authorization"],
    default_timeout_ms: 30000,
    base: Some(("origin", ORIGIN)),
    ..ParitySpec::DEFAULT
};

/// Not UTF-8 (0xff, 0xfe, 0x80, 0xc3 0x28): a recorded text body cannot carry it.
const BINARY: [u8; 12] = [0x25, 0x50, 0x44, 0x46, 0x00, 0xff, 0xfe, 0x80, 0xc3, 0x28, 0x0d, 0x0a];
const BINARY_BASE64: &str = "JVBERgD//oDDKA0K";

const VALIDATION_ONLY: [&str; 4] =
    ["unknown-action", "validation-missing-recipient", "validation-unknown-param", "validation-wrong-type"];
const NOT_CONFIGURED: [&str; 2] = ["not-configured-missing-token", "not-configured-without-credentials"];

fn adapted(fixture: &Value) -> Value {
    let mut fixture = fixture.clone();
    if let Some(base) = fixture["env"]["DOCUSIGN_BASE_URL"].as_str().map(str::to_string) {
        let path = base.strip_prefix(ORIGIN).expect("the recorded base is under the origin");
        fixture["env"]["DOCUSIGN_BASE_PATH"] = json!(path);
    }
    fixture
}

fn replayed() -> Vec<Value> {
    lolo_parity::fixtures(&SPEC)
        .iter()
        .filter(|f| {
            let case = lolo_parity::text(f, "case");
            !VALIDATION_ONLY.contains(&case) && !NOT_CONFIGURED.contains(&case)
        })
        .map(adapted)
        .collect()
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = replayed();
    assert_eq!(all.len(), 13);
    for fixture in &all {
        lolo_parity::check(&SPEC, fixture);
    }
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_binary_document_reaches_the_result_as_base64_of_its_bytes() {
    let mut fixture = adapted(&lolo_parity::fixture_named(&SPEC, "download-document"));
    fixture["outcome"]["data"]["content"] = json!(BINARY_BASE64);
    let step = ReplayStep {
        exchanges: vec![ExchangeContract {
            request_body: RecordedBody::RecordedText,
            response_body: RecordedBody::Bytes(BINARY.to_vec()),
            timeout: Deadline::Default,
        }],
        ..ReplayStep::default()
    };
    ParitySession::new(&SPEC, &fixture, FixtureAuthority::default()).check(&fixture, step);
}

#[test]
#[should_panic(expected = "emitted result")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn control_a_binary_document_does_not_match_the_text_fixture_content() {
    let fixture = adapted(&lolo_parity::fixture_named(&SPEC, "download-document"));
    let step = ReplayStep {
        exchanges: vec![ExchangeContract {
            request_body: RecordedBody::RecordedText,
            response_body: RecordedBody::Bytes(BINARY.to_vec()),
            timeout: Deadline::Default,
        }],
        ..ReplayStep::default()
    };
    ParitySession::new(&SPEC, &fixture, FixtureAuthority::default()).check(&fixture, step);
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    let rejected: Vec<String> = lolo_parity::fixtures(&SPEC)
        .iter()
        .filter(|f| !lolo_parity::is_ok(f) && f["outcome"]["code"] == "VALIDATION_ERROR")
        .map(|f| lolo_parity::text(f, "case").to_string())
        .collect();
    assert_eq!(rejected.len(), VALIDATION_ONLY.len());
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_missing_credential_is_refused_before_any_request_is_sent() {
    for case in NOT_CONFIGURED {
        let fixture = adapted(&lolo_parity::fixture_named(&SPEC, case));
        let run = lolo_parity::run(&SPEC, &fixture);
        assert!(run.sent.is_empty(), "{case}: nothing is sent");
        assert!(run.results.is_empty(), "{case}: no result");
        assert_eq!(run.failures.len(), 1, "{case}: one failure");
        assert_eq!(run.failures[0]["code"], Value::Null, "{case}");
    }
}

#[test]
#[should_panic(expected = "url")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = adapted(&lolo_parity::fixture_named(&SPEC, "get-envelope-status-completed"));
    fixture["exchanges"][0]["request"]["url"] = json!(format!("{ORIGIN}/restapi/v2.1/accounts/acct-1/envelopes/env-2"));
    lolo_parity::check(&SPEC, &fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = adapted(&lolo_parity::fixture_named(&SPEC, "send-envelope"));
    fixture["outcome"]["data"]["status"] = json!("created");
    lolo_parity::check(&SPEC, &fixture);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
