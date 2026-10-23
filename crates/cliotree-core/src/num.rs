//! A number that is either an exact integer or a float, with exact comparison.

use std::cmp::Ordering;

/// A bound or value that keeps the difference between `Int` and `Float`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    /// Compares two numbers exactly, with no loss of precision for large
    /// integers. Returns `None` when either side is NaN.
    pub fn compare(self, other: Num) -> Option<Ordering> {
        match (self, other) {
            (Num::Int(a), Num::Int(b)) => Some(a.cmp(&b)),
            (Num::Float(a), Num::Float(b)) => a.partial_cmp(&b),
            (Num::Int(a), Num::Float(b)) => cmp_int_float(a, b),
            (Num::Float(a), Num::Int(b)) => cmp_int_float(b, a).map(Ordering::reverse),
        }
    }
}

impl std::fmt::Display for Num {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Num::Int(i) => write!(f, "{i}"),
            Num::Float(x) => write!(f, "{x}"),
        }
    }
}

/// Compares the integer `i` with the float `f`.
fn cmp_int_float(i: i64, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    // 2^63 is exactly representable; every i64 is below it.
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    if f >= TWO_63 {
        return Some(Ordering::Less);
    }
    if f < -TWO_63 {
        return Some(Ordering::Greater);
    }
    let truncated = f.trunc();
    // In range by the two checks above, so the cast is exact.
    match i.cmp(&(truncated as i64)) {
        Ordering::Equal => {
            let fraction = f - truncated;
            if fraction > 0.0 {
                Some(Ordering::Less)
            } else if fraction < 0.0 {
                Some(Ordering::Greater)
            } else {
                Some(Ordering::Equal)
            }
        }
        other => Some(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Ordering::*;

    #[test]
    fn integers_compare_exactly_beyond_float_precision() {
        // 2^53 + 1 is not representable as a float; the float 2^53 must still
        // compare below the integer.
        let big = (1i64 << 53) + 1;
        assert_eq!(
            Num::Int(big).compare(Num::Float((1i64 << 53) as f64)),
            Some(Greater)
        );
        assert_eq!(
            Num::Float((1i64 << 53) as f64).compare(Num::Int(big)),
            Some(Less)
        );
    }

    #[test]
    fn fractions_and_extremes() {
        assert_eq!(Num::Int(1).compare(Num::Float(1.5)), Some(Less));
        assert_eq!(Num::Int(2).compare(Num::Float(1.5)), Some(Greater));
        assert_eq!(Num::Int(-1).compare(Num::Float(-1.5)), Some(Greater));
        assert_eq!(Num::Int(3).compare(Num::Float(3.0)), Some(Equal));
        assert_eq!(
            Num::Int(i64::MAX).compare(Num::Float(f64::INFINITY)),
            Some(Less)
        );
        assert_eq!(
            Num::Int(i64::MIN).compare(Num::Float(f64::NEG_INFINITY)),
            Some(Greater)
        );
        assert_eq!(
            Num::Int(i64::MIN).compare(Num::Float(-9_223_372_036_854_775_808.0)),
            Some(Equal)
        );
        assert_eq!(
            Num::Int(i64::MAX).compare(Num::Float(9_223_372_036_854_775_808.0)),
            Some(Less)
        );
    }

    #[test]
    fn nan_is_unordered() {
        assert_eq!(Num::Int(1).compare(Num::Float(f64::NAN)), None);
        assert_eq!(Num::Float(f64::NAN).compare(Num::Float(1.0)), None);
    }
}
