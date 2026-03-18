use std::time::Instant;

/// Redraw interval for spinner animation.
pub const TICK_MS: u64 = 250;

/// Duration of one full rotation in seconds.
const ROTATION_PERIOD_S: f32 = 2.0;

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Returns the current spinner rotation angle in radians (continuous).
pub fn angle() -> f32 {
    let start = START.get_or_init(Instant::now);
    let elapsed = start.elapsed().as_secs_f32();
    std::f32::consts::TAU * (elapsed / ROTATION_PERIOD_S)
}
