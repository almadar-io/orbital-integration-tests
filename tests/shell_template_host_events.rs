//! The compiled web host handles the system UI events (`@almadar/core` `SYSTEM_UI_EVENTS`):
//! each shell template mounts its `UI:SIGN_OUT` listener inside the event bus, where the
//! rendered components emit (G-PATTERN-SYNC-031). The listener's behavior is tested in the
//! template itself (`features/auth/__tests__/SignOutListener.test.tsx`).

use std::path::Path;

#[test]
fn each_shell_mounts_its_sign_out_listener_inside_the_event_bus() {
    for shell in ["almadar-shell", "almadar-shell-hono"] {
        let client = Path::new(env!("CARGO_MANIFEST_DIR")).join("../orbital-shell-typescript/templates").join(shell).join("packages/client/src");
        let app = std::fs::read_to_string(client.join("App.tsx")).unwrap();
        let bus = app.find("<EventBusProvider>").unwrap_or_else(|| panic!("{shell}: no event bus"));
        let close = app.find("</EventBusProvider>").unwrap();
        let listener = app.find("<SignOutListener />").unwrap_or_else(|| panic!("{shell}: no sign-out listener"));
        assert!(bus < listener && listener < close, "{shell}: the listener sits inside the bus");
        let source = std::fs::read_to_string(client.join("features/auth/SignOutListener.tsx")).unwrap();
        assert!(source.contains("bus.on('UI:SIGN_OUT'") && source.contains("signOut()"), "{shell}: the listener ends the session");
    }
}
