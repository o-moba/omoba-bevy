//! Monotonic simulation clock for native hosts and browser practice.
//!
//! Native code retains the exact standard-library type. In a browser the engine
//! supplies performance.now(), where std::time::Instant::now() is unsupported.
#[cfg(target_arch = "wasm32")]
pub use bevy::platform::time::Instant;
#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #[test]
    fn native_clock_is_exactly_the_existing_standard_type() {
        let now: std::time::Instant = super::Instant::now();
        let alias: super::Instant = now;
        assert_eq!(alias, now);
    }
}
