// H.6 parity: every recorded metaAds fixture that sends a request or maps a reply, replayed through the kernel
// against `std-meta-ads.lolo`, makes the request the manual provider made and emits the recorded outcome.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-meta-ads.lolo",
    trait_name: "MetaAdsClient",
    fixtures_dir: "metaAds",
    events: &[
        ActionEvents { action: "getSpend", input: "META_ADS_GET_SPEND", result: "META_ADS_SPEND_RESULT", failure: Some("META_ADS_FAILED") },
        ActionEvents { action: "listCampaigns", input: "META_ADS_LIST_CAMPAIGNS", result: "META_ADS_CAMPAIGNS_RESULT", failure: Some("META_ADS_FAILED") },
    ],
    env_knobs: &[("META_AD_ACCOUNT_ID", "defaultAccountId")],
    secret_env: &["META_ACCESS_TOKEN"],
    compared_headers: &["content-type", "accept", "authorization", "x-api-key", "user-agent"],
    default_timeout_ms: 15000,
    base: None,
};

const VALIDATION_ONLY: [&str; 4] =
    ["unknown-action", "validation-missing-until", "validation-unknown-param", "validation-wrong-type"];

// The provider checks the token before the account; an atom cannot read a secret knob in a guard, so with
// both absent it reports the missing account first. With an account present the missing token reports identically.
const CREDENTIAL_ABSENT: &str = "credential-absent";

#[test]
fn a_missing_token_with_an_account_reports_the_provider_message() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "get-spend");
    fixture["env"] = serde_json::json!({"META_AD_ACCOUNT_ID": "1234567890"});
    fixture["exchanges"] = serde_json::json!([]);
    fixture["outcome"] = serde_json::json!({"ok": false, "code": null, "message": "META_ACCESS_TOKEN not configured"});
    lolo_parity::check(&SPEC, &fixture);
}

#[test]
#[should_panic(expected = "emitted failure")]
fn the_credential_absent_fixture_is_not_replayable_by_the_atom() {
    lolo_parity::check(&SPEC, &lolo_parity::fixture_named(&SPEC, CREDENTIAL_ABSENT));
}

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 21, "the recorded metaAds cases");
    let mut excluded = VALIDATION_ONLY.to_vec();
    excluded.push(CREDENTIAL_ABSENT);
    assert_eq!(lolo_parity::replay_all_except(&SPEC, &excluded), 16);
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    lolo_parity::drifted_url_control(&SPEC, "get-spend", "level=account", "level=ad");
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "get-spend", "spend", serde_json::json!(1.0));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
