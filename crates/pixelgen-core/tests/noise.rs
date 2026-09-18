//! The looping guarantees the whole animation design rests on.

use pixelgen_core::noise;

/// Phase 0 and phase 1 must sample identical values, or every effect built on
/// this noise jumps at the loop point.
#[test]
fn loop_is_periodic_in_time() {
    for (x, y) in [(0.0, 0.0), (3.5, 7.25), (-2.1, 11.9)] {
        let a = noise::looped(x, y, 0.0, 0.05, 1.3, 42);
        let b = noise::looped(x, y, 1.0, 0.05, 1.3, 42);
        assert!((a - b).abs() < 1e-5, "looped({x},{y}): t=0 gave {a} but t=1 gave {b}");
    }
}

/// Drifting relies on the lattice wrapping, so a shift of exactly one period
/// must land on the same value.
#[test]
fn value4_tiling_wraps_on_its_period() {
    const PERIOD: i32 = 4;
    for x in [0.0, 0.3, 1.75, 3.9f32] {
        let a = noise::value4_tiling(x, 1.2, 0.5, -0.25, PERIOD, PERIOD, 7);
        let b = noise::value4_tiling(x + PERIOD as f32, 1.2, 0.5, -0.25, PERIOD, PERIOD, 7);
        assert!((a - b).abs() < 1e-5, "x={x}: {a} != {b} after a full period shift");
    }
}

#[test]
fn drift_fbm_is_periodic_in_time() {
    let a = noise::drift_fbm(5.0, 9.0, 0.0, 0.04, 4, 1.0, 0.0, 0.8, 3, 11);
    let b = noise::drift_fbm(5.0, 9.0, 1.0, 0.04, 4, 1.0, 0.0, 0.8, 3, 11);
    assert!((a - b).abs() < 1e-5, "t=0 gave {a} but t=1 gave {b}");
}

#[test]
fn value4_stays_in_the_unit_range() {
    for i in 0..2000 {
        let f = i as f32;
        let v = noise::value4(f * 0.37, f * 0.11, f * 0.03, f * 0.7, 3);
        assert!((0.0..1.0).contains(&v), "value4 returned {v}, outside [0,1)");
    }
}

/// Per-element randomness comes from a hash, not a live generator, so that
/// frames can be rendered out of order and still agree.
#[test]
fn hash01_is_stable_and_in_range() {
    for i in 0..500 {
        let v = noise::hash01(i, i * 7, 3, 0, 9);
        assert!((0.0..1.0).contains(&v), "hash01 returned {v}");
        assert_eq!(v, noise::hash01(i, i * 7, 3, 0, 9), "hash01 is not deterministic");
    }
}
