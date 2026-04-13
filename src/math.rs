/// Numerically stable `log(exp(a) + exp(b))`.
///
/// Handles `NEG_INFINITY` inputs so it can be used as a fold identity for
/// computing log-sum-exp over a slice.
pub fn logaddexp(a: f64, b: f64) -> f64 {
    if a == f64::NEG_INFINITY {
        return b;
    }
    if b == f64::NEG_INFINITY {
        return a;
    }
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    hi + (-(hi - lo)).exp().ln_1p()
}
