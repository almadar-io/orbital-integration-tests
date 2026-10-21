// Every maps fixture (live Nominatim exchanges recorded 2026-10-17, plus the network-free haversine case), replayed
// through the kernel against `std-maps.lolo`, makes the recorded request and emits the recorded outcome.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/connect/atoms/std-maps.lolo",
    trait_name: "MapsClient",
    fixtures_dir: "maps",
    events: &[
        ActionEvents {
            action: "geocode",
            input: "GEOCODE",
            result: "GEOCODED",
            failure: Some("MAPS_FAILURE"),
            data_field: None,
        },
        ActionEvents {
            action: "reverseGeocode",
            input: "REVERSE_GEOCODE",
            result: "REVERSE_GEOCODED",
            failure: Some("MAPS_FAILURE"),
            data_field: None,
        },
        ActionEvents {
            action: "distance",
            input: "DISTANCE",
            result: "DISTANCE_COMPUTED",
            failure: Some("MAPS_FAILURE"),
            data_field: None,
        },
    ],
    compared_headers: &["accept", "user-agent"],
    default_timeout_ms: 6000,
    failure_message_field: "error",
    ..ParitySpec::DEFAULT
};

#[test]
fn every_recorded_outcome_is_replayed_by_the_atom() {
    assert_eq!(
        lolo_parity::fixtures(&SPEC).len(),
        5,
        "the recorded maps cases"
    );
    assert_eq!(
        lolo_parity::replay_all_except(&SPEC, &[]),
        5,
        "every case has a recorded outcome"
    );
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    lolo_parity::drifted_url_control(&SPEC, "geocode", "limit=1", "limit=2");
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(
        &SPEC,
        "distance-paris-london",
        "distanceKm",
        serde_json::json!(343.558),
    );
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
