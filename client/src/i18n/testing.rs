//! Tests that need the process-wide language switched.
//!
//! `tr` reads one process-wide locale, and the test harness runs tests on
//! parallel threads of one process, so a test must never switch it in
//! place: every other test asserting English copy could observe it. Such a
//! test re-runs itself alone in a child process of the same test binary:
//!
//! ```ignore
//! #[test]
//! fn home_screen_relabels_in_chinese() {
//!     if crate::i18n::testing::isolated("frontend::home::tests::home_screen_relabels_in_chinese") {
//!         return; // the parent: the child passed
//!     }
//!     // The child: free to use `I18nPlugin::fixed(..)` or `Locale::set`
//!     // on the plugin's resource.
//! }
//! ```
//!
//! Tests that only need `Localized` relabelling do not need this: the relabel
//! system follows the `Locale` resource, and `Locale::detached` never touches
//! the process-wide value.
use std::process::Command;

const CHILD_ENV: &str = "OMOBA_I18N_ISOLATED_TEST";

/// In the parent, runs `test_path` (the test's full path inside the crate,
/// e.g. `pause_menu::tests::language_row_…`) alone in a child process of this
/// test binary, asserts that it passed, and returns `true`. In that child it
/// returns `false`, and the caller runs its body.
pub(crate) fn isolated(test_path: &str) -> bool {
    if std::env::var(CHILD_ENV).is_ok_and(|running| running == test_path) {
        return false;
    }
    let binary = std::env::current_exe().expect("test binary path");
    let output = Command::new(binary)
        .args([test_path, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, test_path)
        .output()
        .expect("spawn the isolated test process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "isolated test {test_path} failed:\n{stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("1 passed"),
        "isolated test {test_path} did not run (wrong path?):\n{stdout}"
    );
    true
}
