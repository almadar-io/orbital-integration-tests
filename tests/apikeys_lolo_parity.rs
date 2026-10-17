// Parity for apikeys: the kernel mints a key through `std-api-keys.lolo` (random-bytes, hash/scrypt over the secret salt,
// persist) and the hash it stores is the one `@almadar/auth` computes: the fixtures pin the plaintext a
// fixed draw derives and the `hashApiKey` of it (computed with node's scryptSync over the hex salt), and
// `packages/almadar-auth/test/api-key.test.ts` pins the same constants on the verify side.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySession, ParitySpec, ReplayStep};
use serde_json::Value;

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-api-keys.lolo",
    trait_name: "ApiKeyMinter",
    fixtures_dir: "apikeys",
    events: &[ActionEvents {
        action: "mint",
        input: "MINT_KEY",
        result: "API_KEY_MINTED",
        failure: Some("API_KEY_FAILED"),
        data_field: None,
    }],
    secret_env: &["ALMADAR_API_KEY_SALT"],
    ..ParitySpec::DEFAULT
};

fn step(fixture: &Value) -> ReplayStep {
    let entropy = fixture["randoms"]
        .as_array()
        .expect("randoms")
        .iter()
        .map(|bytes| {
            bytes
                .as_array()
                .expect("a byte array")
                .iter()
                .map(|b| u8::try_from(b.as_u64().expect("byte")).expect("byte"))
                .collect()
        })
        .collect();
    ReplayStep { entropy, ..ReplayStep::default() }
}

fn replay(fixture: &Value) -> (lolo_parity::Run, Vec<Value>) {
    let mut session = ParitySession::new(&SPEC, fixture, lolo_parity::FixtureAuthority::default());
    let run = session.check(fixture, step(fixture));
    let rows = session.rows("ApiKeyRecord");
    (run, rows)
}

#[test]
fn every_recorded_case_mints_the_recorded_key_or_fails_with_the_recorded_cause() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 5);
    for fixture in &all {
        let (_, rows) = replay(fixture);
        match fixture.get("row") {
            Some(row) => assert_eq!(rows, vec![row.clone()], "{}: the persisted row", fixture["case"]),
            None => assert!(rows.is_empty(), "{}: a failed mint persists nothing: {rows:?}", fixture["case"]),
        }
    }
}

#[test]
fn control_a_drifted_hash_or_a_different_draw_is_not_the_recorded_key() {
    let fixture = lolo_parity::fixture_named(&SPEC, "mint-fixed-entropy");
    let mut other_draw = step(&fixture);
    other_draw.entropy = vec![vec![0xcd; 32]];
    let mut session = ParitySession::new(&SPEC, &fixture, lolo_parity::FixtureAuthority::default());
    let run = session.run(&fixture, other_draw);
    assert_eq!(run.results.len(), 1);
    assert_ne!(run.results[0]["keyHash"], fixture["outcome"]["data"]["keyHash"]);
    assert_ne!(run.results[0]["plaintext"], fixture["outcome"]["data"]["plaintext"]);
    assert!(run.results[0]["plaintext"].as_str().unwrap().starts_with("sk_live_"));
    assert_eq!(run.results[0]["lastFour"], run.results[0]["plaintext"].as_str().unwrap()[run.results[0]["plaintext"].as_str().unwrap().len() - 4..]);
    let mut drifted = fixture.clone();
    drifted["outcome"]["data"]["keyHash"] = Value::String("0".repeat(64));
    let outcome = std::panic::catch_unwind(|| replay(&drifted));
    assert!(outcome.is_err(), "a recorded hash that is not the kernel's must fail the replay");
}

#[test]
fn the_plaintext_is_never_stored_and_a_bad_salt_persists_nothing() {
    let fixture = lolo_parity::fixture_named(&SPEC, "mint-fixed-entropy");
    let (run, rows) = replay(&fixture);
    let plaintext = run.results[0]["plaintext"].as_str().unwrap();
    assert!(!rows.iter().any(|row| row.to_string().contains(plaintext)), "{rows:?}");
    assert!(!rows.iter().any(|row| row.to_string().contains("00112233445566778899")), "the salt value is never in a row");
    for case in ["salt-not-hex", "salt-odd-length", "salt-missing"] {
        let fixture = lolo_parity::fixture_named(&SPEC, case);
        let (_, rows) = replay(&fixture);
        assert!(rows.is_empty(), "{case}");
    }
}
