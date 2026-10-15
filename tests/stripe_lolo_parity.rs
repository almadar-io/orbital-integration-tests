// H.6 parity: every recorded stripe fixture the atom can express, replayed through the kernel against
// `std-stripe.lolo`, makes the request the manual provider made and emits the recorded outcome.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-stripe.lolo",
    trait_name: "StripeClient",
    fixtures_dir: "stripe",
    events: &[
        ActionEvents { action: "createPaymentIntent", input: "STRIPE_CREATE_PAYMENT_INTENT", result: "STRIPE_CREATE_PAYMENT_INTENT_RESULT", failure: Some("STRIPE_FAILED") },
        ActionEvents { action: "confirmPayment", input: "STRIPE_CONFIRM_PAYMENT", result: "STRIPE_CONFIRM_PAYMENT_RESULT", failure: Some("STRIPE_FAILED") },
        ActionEvents { action: "refund", input: "STRIPE_REFUND", result: "STRIPE_REFUND_RESULT", failure: Some("STRIPE_FAILED") },
        ActionEvents { action: "createCustomer", input: "STRIPE_CREATE_CUSTOMER", result: "STRIPE_CREATE_CUSTOMER_RESULT", failure: Some("STRIPE_FAILED") },
        ActionEvents { action: "createCheckoutSession", input: "STRIPE_CREATE_CHECKOUT_SESSION", result: "STRIPE_CREATE_CHECKOUT_SESSION_RESULT", failure: Some("STRIPE_FAILED") },
        ActionEvents { action: "createPortalSession", input: "STRIPE_CREATE_PORTAL_SESSION", result: "STRIPE_CREATE_PORTAL_SESSION_RESULT", failure: Some("STRIPE_FAILED") },
        ActionEvents { action: "getSubscription", input: "STRIPE_GET_SUBSCRIPTION", result: "STRIPE_GET_SUBSCRIPTION_RESULT", failure: Some("STRIPE_FAILED") },
        ActionEvents { action: "createSubscription", input: "STRIPE_CREATE_SUBSCRIPTION", result: "STRIPE_CREATE_SUBSCRIPTION_RESULT", failure: Some("STRIPE_FAILED") },
        ActionEvents { action: "updateSubscription", input: "STRIPE_UPDATE_SUBSCRIPTION", result: "STRIPE_UPDATE_SUBSCRIPTION_RESULT", failure: Some("STRIPE_FAILED") },
        ActionEvents { action: "cancelSubscription", input: "STRIPE_CANCEL_SUBSCRIPTION", result: "STRIPE_CANCEL_SUBSCRIPTION_RESULT", failure: Some("STRIPE_FAILED") },
    ],
    env_knobs: &[("STRIPE_PRICE_SOLO", "priceSolo"), ("STRIPE_PRICE_TEAMS", "priceTeams")],
    secret_env: &["STRIPE_SECRET_KEY", "STRIPE_WEBHOOK_SECRET"],
    compared_headers: &["content-type", "accept", "authorization", "stripe-version", "idempotency-key"],
    default_timeout_ms: 80_000,
    base: None,
};

// D-A80: contract-validation errors that send nothing.
const VALIDATION_ONLY: [&str; 4] = ["unknown-action", "validation-missing-amount", "validation-unknown-param", "validation-wrong-type"];
// Host policy (H.1): the host owns retries, so the atom sends one request per call.
const RETRIED_BY_THE_HOST: [&str; 4] = [
    "conflict-is-retried",
    "retry-after-header-does-not-lengthen-the-wait",
    "server-error-is-retried-then-succeeds",
    "server-error-retries-exhausted",
];
// Host state: no STRIPE_SECRET_KEY registered means the service is not configured before any call.
const NOT_CONFIGURED: [&str; 2] = ["not-configured-webhook-secret-alone", "not-configured-without-credentials"];
// The typed `Map string string` payload rejects a non-string metadata value at the contract.
const TYPED_AWAY: [&str; 1] = ["create-checkout-session-non-string-metadata"];

fn excluded() -> Vec<&'static str> {
    [&VALIDATION_ONLY[..], &RETRIED_BY_THE_HOST[..], &NOT_CONFIGURED[..], &TYPED_AWAY[..]].concat()
}

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 52, "the recorded stripe cases");
    let skipped = excluded();
    assert_eq!(lolo_parity::replay_all_except(&SPEC, &skipped), all.len() - skipped.len());
}

#[test]
fn the_validation_only_cases_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
fn the_retried_cases_record_more_than_one_exchange() {
    for case in RETRIED_BY_THE_HOST {
        let fixture = lolo_parity::fixture_named(&SPEC, case);
        assert!(fixture["exchanges"].as_array().expect("exchanges").len() > 1, "{case}");
    }
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    lolo_parity::drifted_url_control(&SPEC, "get-subscription", "/v1/subscriptions/sub_1", "/v1/subscriptions/sub_2");
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "get-subscription", "tier", serde_json::json!("teams"));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
