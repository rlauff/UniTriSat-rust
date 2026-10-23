//! Bounded, empirical size-aware estimates. Descriptors are not complexity proofs.
use std::sync::Arc;
use unitrisat::matrix::Matrix;

const BANDS: usize = 32;
const DIMENSIONS: usize = 16;
const BUCKETS: usize = BANDS * DIMENSIONS;
const UNKNOWN: u16 = u16::MAX;

/// Two bytes per input, collected while its coordinates are already parsed.
/// Each band spans two bits of vertex-count times bounding-box lattice capacity.
pub(crate) fn descriptor(vertices: &Matrix) -> u16 {
    let dimension = vertices.columns();
    if dimension > DIMENSIONS {
        return UNKNOWN;
    }
    let mut bounds = [(i64::MAX, i64::MIN); DIMENSIONS];
    for row in vertices.iter_rows() {
        for (value, (low, high)) in row.iter().zip(&mut bounds) {
            *low = (*low).min(*value);
            *high = (*high).max(*value);
        }
    }
    let score = (vertices.rows() as f64).log2()
        + bounds
            .into_iter()
            .take(dimension)
            .map(|(lo, hi)| {
                // i128 subtraction avoids overflow even at the two i64 extremes.
                let span = i128::from(hi).saturating_sub(i128::from(lo));
                ((span as f64) + 1.).log2()
            })
            .sum::<f64>();
    if !score.is_finite() || !(0.0..64.0).contains(&score) {
        return UNKNOWN;
    }
    let band = (score / 2.).floor() as usize;
    u16::try_from(
        dimension
            .saturating_sub(1)
            .saturating_mul(BANDS)
            .saturating_add(band),
    )
    .unwrap_or(UNKNOWN)
}

#[derive(Clone, Copy, Default)]
struct Bucket {
    remaining: u64,
    count: u64,
    seconds: f64,
    logs: f64,
    logs2: f64,
    censored: bool,
}
impl Bucket {
    fn sample(&mut self, seconds: f64) {
        if !seconds.is_finite() || seconds <= 0. {
            return;
        }
        let log = seconds.ln();
        self.count = self.count.saturating_add(1);
        self.seconds += seconds;
        self.logs += log;
        self.logs2 += log * log;
    }
    fn variance(self) -> f64 {
        let n = self.count.max(1) as f64;
        (self.logs2 / n - (self.logs / n).powi(2)).max(0.)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Estimate {
    Learning,
    Unpredictable,
    Ready { seconds: f64, rough: bool },
}

pub(crate) struct Estimator {
    ids: Arc<[u16]>,
    buckets: Vec<Bucket>,
    unknown: u64,
    observed: Bucket,
}
impl Estimator {
    pub(crate) fn new(ids: Arc<[u16]>) -> Self {
        let mut estimator = Self {
            ids,
            buckets: vec![Bucket::default(); BUCKETS],
            unknown: 0,
            observed: Bucket::default(),
        };
        for &id in estimator.ids.iter() {
            if let Some(bucket) = estimator.buckets.get_mut(usize::from(id)) {
                bucket.remaining = bucket.remaining.saturating_add(1);
            } else {
                estimator.unknown = estimator.unknown.saturating_add(1);
            }
        }
        estimator
    }
    fn id(&self, index: u64) -> Option<usize> {
        usize::try_from(index.checked_sub(1)?)
            .ok()
            .and_then(|i| self.ids.get(i))
            .copied()
            .map(usize::from)
    }
    /// Retire saved work without treating its unrecorded topology as a sample.
    pub(crate) fn restore_completed(&mut self, index: u64) {
        if let Some(bucket) = self.id(index).and_then(|id| self.buckets.get_mut(id)) {
            bucket.remaining = bucket.remaining.saturating_sub(1);
        } else {
            self.unknown = self.unknown.saturating_sub(1);
        }
    }
    /// Retire current work; censored/failed jobs pass no sample.
    pub(crate) fn finish(&mut self, index: u64, seconds: Option<f64>) {
        self.restore_completed(index);
        if let Some(seconds) = seconds {
            self.observed.sample(seconds);
        } else {
            self.observed.censored = true;
        }
        if let Some(bucket) = self.id(index).and_then(|id| self.buckets.get_mut(id)) {
            if let Some(seconds) = seconds {
                bucket.sample(seconds);
            } else {
                bucket.censored = true;
            }
        }
    }
    /// Fixed 512-bucket work plus active workers, independent of input count.
    pub(crate) fn estimate(
        &self,
        active: impl IntoIterator<Item = (u64, f64)>,
        workers: usize,
    ) -> Estimate {
        if self.unknown == 0 && self.buckets.iter().all(|b| b.remaining == 0) {
            return Estimate::Ready {
                seconds: 0.,
                rough: false,
            };
        }
        // Sparse, noisy or censored evidence still gives a useful scale, but
        // there is no duration forecast before the first usable completion.
        if self.observed.count == 0 {
            return if self.observed.censored {
                Estimate::Unpredictable
            } else {
                Estimate::Learning
            };
        }
        let average = self.observed.seconds / (self.observed.count as f64);
        // The extra slot covers descriptors outside the supported size range.
        let mut predictions = vec![None; BUCKETS + 1];
        let mut rough = self.unknown > 0;
        let mut work = 0.;
        for (dimension, buckets) in self.buckets.chunks_exact(BANDS).enumerate() {
            let model = Model::fit(buckets);
            let nearest = nearest_samples(buckets);
            for (band, bucket) in buckets.iter().enumerate() {
                if bucket.remaining == 0 {
                    continue;
                }
                rough |= bucket.censored;
                let predicted = if bucket.count > 0 {
                    rough |= bucket.count < 3 || bucket.variance() > 2_f64.ln().powi(2);
                    bucket.seconds / (bucket.count as f64)
                } else {
                    rough = true;
                    // Keep size trends for sorted inputs, even beyond the
                    // observed range. Noisy/inverted distant trends fall back
                    // to the nearest size; unseen dimensions use the run mean.
                    let fallback = nearest
                        .get(band)
                        .copied()
                        .flatten()
                        .and_then(|i| buckets.get(i))
                        .map_or(average, |sample| sample.seconds / (sample.count as f64));
                    model.and_then(|m| m.predict(band)).unwrap_or(fallback)
                };
                if !predicted.is_finite() || predicted <= 0. {
                    return Estimate::Unpredictable;
                }
                if let Some(slot) =
                    predictions.get_mut(dimension.saturating_mul(BANDS).saturating_add(band))
                {
                    *slot = Some(predicted);
                }
                work += predicted * (bucket.remaining as f64);
            }
        }
        if self.unknown > 0 {
            if let Some(slot) = predictions.get_mut(BUCKETS) {
                *slot = Some(average);
            }
            work += average * (self.unknown as f64);
        }
        let mut active_counts = vec![0_u64; BUCKETS + 1];
        let mut straggler = 0_f64;
        for (index, age) in active {
            let Some(id) = self.id(index) else {
                continue;
            };
            let id = id.min(BUCKETS);
            let Some(predicted) = predictions.get(id).copied().flatten() else {
                continue;
            };
            let Some(count) = active_counts.get_mut(id) else {
                continue;
            };
            *count = count.saturating_add(1);
            if !age.is_finite() {
                return Estimate::Unpredictable;
            }
            let age = age.max(0.);
            if age > predicted {
                rough = true;
            }
            // Age is a lower bound on an unfinished duration, not a completed
            // sample. Keep a heuristic residual that grows with overdue age;
            // it is deliberately labelled low confidence, not a time bound.
            let residual = (predicted - age).max(predicted.max(age) * 0.25);
            work = work - predicted + residual;
            straggler = straggler.max(residual);
        }
        // A final active straggler uses residual duration, not its original total.
        let mut largest = 0_f64;
        for ((remaining, count), prediction) in self
            .buckets
            .iter()
            .map(|b| b.remaining)
            .chain(std::iter::once(self.unknown))
            .zip(active_counts)
            .zip(predictions)
        {
            if remaining > count {
                largest = largest.max(prediction.unwrap_or(0.));
            }
        }
        let seconds = (work / (workers.max(1) as f64))
            .max(straggler)
            .max(largest)
            .max(0.);
        if seconds.is_finite() {
            Estimate::Ready { seconds, rough }
        } else {
            Estimate::Unpredictable
        }
    }
}

// Two linear passes keep nearest-size fallback bounded by the bucket count.
fn nearest_samples(buckets: &[Bucket]) -> [Option<usize>; BANDS] {
    let mut nearest = [None; BANDS];
    let mut previous = None;
    for (band, (bucket, slot)) in buckets.iter().zip(&mut nearest).enumerate() {
        if bucket.count > 0 {
            previous = Some(band);
        }
        *slot = previous;
    }
    let mut next: Option<usize> = None;
    for (band, (bucket, slot)) in buckets.iter().zip(&mut nearest).enumerate().rev() {
        if bucket.count > 0 {
            next = Some(band);
        }
        if let Some(right) = next
            && slot.is_none_or(|left| right.saturating_sub(band) < band.saturating_sub(left))
        {
            *slot = Some(right);
        }
    }
    nearest
}

#[derive(Clone, Copy)]
struct Model {
    intercept: f64,
    slope: f64,
    low: f64,
    high: f64,
    scatter: f64,
    smallest: f64,
}
impl Model {
    fn predict(self, band: usize) -> Option<f64> {
        let x = band as f64;
        if self.scatter > 4_f64.ln().powi(2)
            || ((x < self.low - 2. || x > self.high + 2.) && self.slope <= 0.)
        {
            return None;
        }
        let predicted = (self.intercept + self.slope * x).exp();
        // Degenerate extrapolation must not hide an available empirical mean.
        if !predicted.is_finite() || predicted <= 0. {
            return None;
        }
        Some(if x < self.low - 2. {
            predicted.max(self.smallest / 16.)
        } else {
            predicted
        })
    }
    fn fit(buckets: &[Bucket]) -> Option<Self> {
        let mut n = 0.;
        let mut observations = 0_u64;
        let (mut sx, mut sy, mut xx, mut xy) = (0., 0., 0., 0.);
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut smallest = f64::INFINITY;
        for (band, bucket) in buckets
            .iter()
            .enumerate()
            .filter(|(_, b)| b.count > 0 && !b.censored)
        {
            let x = band as f64;
            let y = bucket.logs / (bucket.count as f64);
            smallest = smallest.min(y.exp());
            n += 1.;
            observations = observations.saturating_add(bucket.count);
            sx += x;
            sy += y;
            xx += x * x;
            xy += x * y;
            low = low.min(x);
            high = high.max(x);
        }
        if n < 2. || observations < 3 || high - low < 1. {
            return None;
        }
        let denominator = n * xx - sx * sx;
        if denominator <= 0. {
            return None;
        }
        let slope = (n * xy - sx * sy) / denominator;
        let intercept = (sy - slope * sx) / n;
        let scatter = buckets
            .iter()
            .enumerate()
            .filter(|(_, b)| b.count > 0 && !b.censored)
            .map(|(band, b)| {
                let error = b.logs / (b.count as f64) - intercept - slope * (band as f64);
                error * error + b.variance()
            })
            .sum::<f64>()
            / n;
        Some(Self {
            intercept,
            slope,
            low,
            high,
            scatter,
            smallest,
        })
    }
}

pub(crate) fn sample(
    status: crate::job::Status,
    timings: Option<crate::job::StageTimes>,
) -> Option<f64> {
    if status != crate::job::Status::Ok {
        return None;
    }
    let t = timings?;
    let seconds = t.lattice + t.formula + t.search;
    (seconds.is_finite() && seconds > 0.).then_some(seconds)
}

#[cfg(test)]
mod tests;
