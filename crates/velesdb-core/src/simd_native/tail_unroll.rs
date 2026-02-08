//! Remainder/tail handling macros for SIMD loops.
//!
//! These macros generate optimal unrolled code for processing the last
//! 1-7 elements that don't fit into a full SIMD lane.

// =============================================================================
// Remainder Handling Macro (FLAG-005: Factorisation)
// =============================================================================

/// Macro for unrolled remainder sum computation (1-7 elements).
/// Generates optimal code for remainders 1-7 with 4->2->1 unrolling.
#[macro_export]
macro_rules! sum_remainder_unrolled_8 {
    ($a:expr, $b:expr, $base:expr, $remainder:expr, $result:expr) => {
        if $remainder >= 4 {
            $result += $a[$base] * $b[$base]
                + $a[$base + 1] * $b[$base + 1]
                + $a[$base + 2] * $b[$base + 2]
                + $a[$base + 3] * $b[$base + 3];
            if $remainder >= 5 {
                $result += $a[$base + 4] * $b[$base + 4];
            }
            if $remainder >= 6 {
                $result += $a[$base + 5] * $b[$base + 5];
            }
            if $remainder == 7 {
                $result += $a[$base + 6] * $b[$base + 6];
            }
        } else if $remainder >= 2 {
            $result += $a[$base] * $b[$base] + $a[$base + 1] * $b[$base + 1];
            if $remainder == 3 {
                $result += $a[$base + 2] * $b[$base + 2];
            }
        } else if $remainder == 1 {
            $result += $a[$base] * $b[$base];
        }
    };
}

/// Macro for unrolled squared L2 remainder (1-7 elements).
#[macro_export]
macro_rules! sum_squared_remainder_unrolled_8 {
    ($a:expr, $b:expr, $base:expr, $remainder:expr, $result:expr) => {
        if $remainder >= 4 {
            let d0 = $a[$base] - $b[$base];
            let d1 = $a[$base + 1] - $b[$base + 1];
            let d2 = $a[$base + 2] - $b[$base + 2];
            let d3 = $a[$base + 3] - $b[$base + 3];
            $result += d0 * d0 + d1 * d1 + d2 * d2 + d3 * d3;
            if $remainder >= 5 {
                let d4 = $a[$base + 4] - $b[$base + 4];
                $result += d4 * d4;
            }
            if $remainder >= 6 {
                let d5 = $a[$base + 5] - $b[$base + 5];
                $result += d5 * d5;
            }
            if $remainder == 7 {
                let d6 = $a[$base + 6] - $b[$base + 6];
                $result += d6 * d6;
            }
        } else if $remainder >= 2 {
            let d0 = $a[$base] - $b[$base];
            let d1 = $a[$base + 1] - $b[$base + 1];
            $result += d0 * d0 + d1 * d1;
            if $remainder == 3 {
                let d2 = $a[$base + 2] - $b[$base + 2];
                $result += d2 * d2;
            }
        } else if $remainder == 1 {
            let d = $a[$base] - $b[$base];
            $result += d * d;
        }
    };
}

// Re-export macros for internal use
#[allow(unused_imports)]
pub(crate) use sum_remainder_unrolled_8;
#[allow(unused_imports)]
pub(crate) use sum_squared_remainder_unrolled_8;
