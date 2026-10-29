// H.6 parity: every recorded push fixture, replayed through the kernel against `std-push.lolo`, makes the request the
// manual provider made (method, URL, compared headers, encrypted body bytes) and emits the recorded outcome. The
// ephemeral key and salt are fed as the journaled Random ask; the request origin comes from the subscription endpoint.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{
    ActionEvents, Deadline, ExchangeContract, FixtureAuthority, ParitySession, ParitySpec, RecordedBody, ReplayStep,
};
use orbital_core::codec::lenient::base64_decode_lenient;
use orbital_core::codec::base64_decode;

fn decode_base64_any(text: &str) -> Vec<u8> {
    base64_decode_lenient(&text.replace('-', "+").replace('_', "/"))
}
use orbital_core::OrbitalSchema;
use serde_json::{json, Value as Json};
use std::path::PathBuf;

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/connect/atoms/std-push.lolo",
    trait_name: "PushClient",
    fixtures_dir: "push",
    events: &[ActionEvents { action: "send", input: "PUSH_SEND", result: "PUSH_SEND_RESULT", failure: Some("PUSH_SEND_FAILED"), data_field: None }],
    env_knobs: &[("VAPID_PUBLIC_KEY", "vapidPublicKey"), ("VAPID_SUBJECT", "vapidSubject")],
    secret_env: &["VAPID_PRIVATE_KEY"],
    compared_headers: &["content-type", "authorization", "ttl", "urgency", "content-encoding", "content-length"],
    default_timeout_ms: 0,
    base: None,
    ..ParitySpec::DEFAULT
};

const VALIDATION_ONLY: [&str; 4] = [
    "unknown-action",
    "validation-missing-title",
    "validation-unknown-param",
    "validation-wrong-type",
];

// The atom fails these before any entropy is asked (subscription guards) or before encryption (no VAPID private key).
const NO_ENTROPY: [&str; 6] = [
    "subscription-empty-endpoint",
    "subscription-with-a-non-string-auth",
    "subscription-without-an-endpoint",
    "subscription-without-keys",
    "vapid-nothing-configured",
    "vapid-private-key-missing",
];

fn schema_for(fixture: &Json) -> OrbitalSchema {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(SPEC.atom);
    let schema = orbital_test_support::lower_lolo_file(&path).unwrap_or_else(|e| panic!("lower: {e}"));
    let mut json = serde_json::to_value(&schema).expect("schema json");
    for orbital in json["orbitals"].as_array_mut().expect("orbitals") {
        for t in orbital["traits"].as_array_mut().into_iter().flatten() {
            for (variable, knob) in SPEC.env_knobs {
                if let Some(raw) = fixture["env"][*variable].as_str() {
                    t["config"][*knob]["default"] = json!(raw);
                }
            }
        }
    }
    serde_json::from_value(json).expect("schema back from json")
}

fn step_for(fixture: &Json) -> ReplayStep {
    let case = lolo_parity::text(fixture, "case");
    let entropy = &fixture["extra"]["entropy"];
    let mut bytes = decode_base64_any(entropy["ephemeralPrivateKey"].as_str().expect("key"));
    bytes.extend(decode_base64_any(entropy["salt"].as_str().expect("salt")));
    let exchanges = fixture["exchanges"]
        .as_array()
        .expect("exchanges")
        .iter()
        .map(|_| ExchangeContract {
            request_body: RecordedBody::Bytes(
                base64_decode(fixture["extra"]["bodyBase64"].as_str().expect("recorded body")).expect("body base64"),
            ),
            timeout: Deadline::Exact(None),
            ..ExchangeContract::default()
        })
        .collect();
    ReplayStep {
        entropy: if NO_ENTROPY.contains(&case) { vec![] } else { vec![bytes] },
        exchanges,
        ..ReplayStep::default()
    }
}

fn replay(fixture: &Json) {
    ParitySession::with_schema(&SPEC, schema_for(fixture), fixture, FixtureAuthority::default())
        .check(fixture, step_for(fixture));
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 41, "the recorded push cases");
    let mut replayed = 0;
    for fixture in all.iter().filter(|f| !VALIDATION_ONLY.contains(&lolo_parity::text(f, "case"))) {
        replay(fixture);
        replayed += 1;
    }
    assert_eq!(replayed, 37);
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
#[should_panic(expected = "url")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "send");
    fixture["exchanges"][0]["request"]["url"] = json!("https://push.example.test/send/abc124");
    replay(&fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "send");
    fixture["outcome"]["data"]["statusCode"] = json!(202);
    replay(&fixture);
}

#[test]
#[should_panic(expected = "request body bytes")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_ciphertext_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "send");
    fixture["extra"]["entropy"]["salt"] = json!("AAAAAAAAAAAAAAAAAAAAAA");
    replay(&fixture);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets_allowing(
        &SPEC,
        &[
            "qQYsogrPfyiau8LBWmOl_LnbiSC9WkSDjJVO_96vNTM",
            "AAAA",
            "not+url/safe",
            "BECHRcCvIQtR5RDxgMq6mnyWPa1YaZpvW1t9NZYwbseWTts_aglewW-vHOaigsyyCK-7ohOfelkh6FaU5bdvg2s",
            "BECHRcCvIQtR5RDxgMq6mnyWPa1YaZpvW1t9NZYwbseWTts_aglewW-vHOaigsyyCK-7ohOfelkh6FaU5bdvg2s==",
        ],
    );
}
