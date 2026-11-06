//! What the processor has, for the loops built for more than the target's baseline.

use core::sync::atomic::{AtomicU8, Ordering};

/// 0 until asked, then 1 without BMI2, 2 with it.
static BMI2: AtomicU8 = AtomicU8::new(0);

/// Whether the processor has BMI2, asked of `cpuid` once. BMI2 needs nothing of the operating
/// system, its instructions using the general registers only. Without `std`, so by `cpuid`
/// itself, not `is_x86_feature_detected!`.
#[inline(always)]
pub(crate) fn has_bmi2() -> bool {
    match BMI2.load(Ordering::Relaxed) {
        0 => detect_bmi2(),
        known => known == 2,
    }
}

#[cold]
#[inline(never)]
fn detect_bmi2() -> bool {
    use core::arch::x86_64::{__cpuid, __cpuid_count};
    // Miri runs no `cpuid`; it checks the build for any x86-64.
    if cfg!(miri) {
        BMI2.store(1, Ordering::Relaxed);
        return false;
    }
    #[allow(unused_unsafe)]
    // SAFETY: every x86-64 processor has `cpuid`; leaf 7 is asked only where leaf 0 says it
    // is there.
    let bmi2 = unsafe { __cpuid(0).eax >= 7 && __cpuid_count(7, 0).ebx & (1 << 8) != 0 };
    BMI2.store(if bmi2 { 2 } else { 1 }, Ordering::Relaxed);
    bmi2
}
