//! The native device scripts (`scripts/native`) expose every lifecycle step.

fn script_file(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/native")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn ios_device_script_exposes_every_lifecycle_subcommand_and_owns_simctl() {
    let script = script_file("ios-device.sh");
    for sub in [
        "boot)",
        "install)",
        "launch)",
        "terminate)",
        "screenshot)",
        "forward)",
    ] {
        assert!(script.contains(sub), "ios-device.sh lacks {sub}");
    }
    assert!(script.contains("ORB_SIM_DEVICE"));
    assert!(script.contains("SIMCTL_CHILD_"));
}

#[test]
fn android_device_script_exposes_every_lifecycle_subcommand_and_owns_adb() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/native/android-device.sh");
    let script = std::fs::read_to_string(path).unwrap();
    for sub in [
        "boot)",
        "install)",
        "launch)",
        "terminate)",
        "screenshot)",
        "forward)",
    ] {
        assert!(script.contains(sub), "android-device.sh lacks {sub}");
    }
    assert!(script.contains("ORB_AVD:-orbital-test"));
    assert!(script.contains("adb forward"));
    assert!(script.contains("--es"));
}
