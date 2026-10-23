//! Conservative, exact bounds for every generated integer intermediate.

/// Smallest safe CUDA integer type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Signed 32-bit arithmetic.
    I32,
    /// Signed 64-bit arithmetic.
    I64,
}

/// Select a safe integer lane for dimensions 2 through 7.
///
/// Returns `None` for malformed matrices, unsupported dimensions, or bounds
/// above i64. Bounds use checked u128 arithmetic: overflow means CPU fallback.
/// For a k-column minor every partial cofactor sum is bounded by k! times
/// the product of its column spreads; every vertex projection is bounded
/// by the sum of the corresponding absolute-coordinate times normal bounds.
/// This also bounds intermediate products, differences, and stored inputs.
/// The factorial estimate is deliberately more conservative than Hadamard's
/// bound on a *finished* determinant, which alone does not bound partial sums.
pub fn precision(points: &[i64], dimension: usize) -> Option<Lane> {
    if !(2..=7).contains(&dimension) || points.len().checked_rem(dimension)? != 0 {
        return None;
    }
    let mut spreads = Vec::with_capacity(dimension);
    let mut magnitudes = Vec::with_capacity(dimension);
    let mut bound = 1_u128;
    for column in 0..dimension {
        let mut lo = i64::MAX;
        let mut hi = i64::MIN;
        let mut magnitude = 0_u128;
        for row in points.chunks_exact(dimension) {
            let value = *row.get(column)?;
            lo = lo.min(value);
            hi = hi.max(value);
            magnitude = magnitude.max(u128::from(value.unsigned_abs()));
        }
        let spread = if points.is_empty() {
            0
        } else {
            u128::try_from(i128::from(hi).checked_sub(i128::from(lo))?).ok()?
        };
        bound = bound.max(spread).max(magnitude);
        spreads.push(spread);
        magnitudes.push(magnitude);
    }
    // All orders occur in the subset-DP determinant expansion. Check each
    // subset, including zero-spread inputs where lower minors dominate.
    let end = 1_usize.checked_shl(u32::try_from(dimension).ok()?)?;
    let mut projection = 0_u128;
    for mask in 1..end.checked_sub(1)? {
        let mut minor = 1_u128;
        let mut order = 0_u128;
        let mut omitted = None;
        for (column, &spread) in spreads.iter().enumerate() {
            if mask & 1_usize.checked_shl(u32::try_from(column).ok()?)? != 0 {
                order = order.checked_add(1)?;
                minor = minor.checked_mul(order)?.checked_mul(spread)?;
            } else {
                omitted = Some(column);
            }
        }
        bound = bound.max(minor);
        if usize::try_from(order).ok()? == dimension.checked_sub(1)? {
            projection = projection.checked_add(minor.checked_mul(*magnitudes.get(omitted?)?)?)?;
        }
    }
    bound = bound.max(projection);
    if bound <= u128::try_from(i32::MAX).ok()? {
        Some(Lane::I32)
    } else if bound <= u128::try_from(i64::MAX).ok()? {
        Some(Lane::I64)
    } else {
        None
    }
}
