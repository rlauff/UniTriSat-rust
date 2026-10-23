//! The only unsafe operations are cudarc library probing and kernel launch.
#![allow(unsafe_code)]

use crate::{Backend, Lane, Search, precision, source};
use cudarc::driver::{CudaContext, CudaModule, CudaSlice, CudaStream, LaunchConfig, PushKernelArg};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

const TILE: u64 = 16;
const BATCH_TILES: u64 = 4096;
type Modules = BTreeMap<(usize, bool), Arc<CudaModule>>;
static CONTEXT: OnceLock<Result<Arc<CudaContext>, String>> = OnceLock::new();
static MODULES: Mutex<Modules> = Mutex::new(BTreeMap::new());

#[cfg(test)]
std::thread_local! {
    static FAIL_AFTER: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

fn guarded<T>(operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
        .unwrap_or_else(|_| Err("CUDA dependency panicked".to_owned()))
}

fn context() -> Result<Arc<CudaContext>, String> {
    CONTEXT
        .get_or_init(|| {
            guarded(|| {
                // SAFETY: probes load the platform CUDA libraries without dereferencing
                // user pointers. As with cudarc itself, installed driver libraries are
                // trusted. Probing avoids cudarc's missing-library panic on CPU hosts.
                let available = unsafe {
                    cudarc::driver::sys::is_culib_present()
                        && cudarc::nvrtc::sys::is_culib_present()
                };
                if !available {
                    return Err("CUDA driver or NVRTC library unavailable".to_owned());
                }
                CudaContext::new(0).map_err(|e| format!("CUDA device unavailable: {e}"))
            })
        })
        .clone()
}

fn module(ctx: &Arc<CudaContext>, dimension: usize, lane: Lane) -> Result<Arc<CudaModule>, String> {
    let mut modules = MODULES.lock().unwrap_or_else(PoisonError::into_inner);
    let key = (dimension, lane == Lane::I64);
    if let Some(module) = modules.get(&key) {
        return Ok(Arc::clone(module));
    }
    let text = source::generate(dimension, lane).ok_or("CUDA source dimension unsupported")?;
    let ptx =
        cudarc::nvrtc::compile_ptx(&text).map_err(|e| format!("NVRTC compilation failed: {e}"))?;
    let module = ctx
        .load_module(ptx)
        .map_err(|e| format!("CUDA module failed: {e}"))?;
    modules.insert(key, Arc::clone(&module));
    Ok(module)
}

struct Job {
    stream: Arc<CudaStream>,
    module: Arc<CudaModule>,
    vertices: CudaSlice<i64>,
    count: u32,
    tiles: u64,
    lane: Lane,
}

impl Job {
    fn new(points: &[i64], dimension: usize, simplices: &[u32]) -> Result<Self, String> {
        let lane = precision(points, dimension).ok_or("CUDA dimension or precision unsupported")?;
        let width = dimension.checked_add(1).ok_or("simplex width overflow")?;
        if simplices.len().checked_rem(width) != Some(0) {
            return Err("invalid simplex shape".to_owned());
        }
        let n = simplices
            .len()
            .checked_div(width)
            .ok_or("simplex count overflow")?;
        let count = u32::try_from(n).map_err(|_| "too many simplices")?;
        let capacity = simplices
            .len()
            .checked_mul(dimension)
            .ok_or("vertex buffer overflow")?;
        // Validate every label before creating a context or emitting any pairs.
        let mut flat = Vec::with_capacity(capacity);
        for &label in simplices {
            let start = usize::try_from(label)
                .ok()
                .and_then(|i| i.checked_mul(dimension))
                .ok_or("point index overflow")?;
            let end = start.checked_add(dimension).ok_or("point index overflow")?;
            flat.extend_from_slice(points.get(start..end).ok_or("invalid simplex label")?);
        }
        if count < 2 {
            return Err("fewer than two simplices".to_owned());
        }
        let ctx = context()?;
        let module = module(&ctx, dimension, lane)?;
        let stream = ctx.new_stream().map_err(|e| e.to_string())?;
        let vertices = stream.clone_htod(&flat).map_err(|e| e.to_string())?;
        let sides = u64::from(count).div_ceil(TILE);
        let tiles = triangular(sides).ok_or("tile count overflow")?;
        Ok(Self {
            stream,
            module,
            vertices,
            count,
            tiles,
            lane,
        })
    }

    fn batch(&self, first: u64, count: u64) -> Result<Vec<u8>, String> {
        let len = usize::try_from(count.checked_mul(256).ok_or("batch size overflow")?)
            .map_err(|_| "batch size overflow")?;
        let mut output = self
            .stream
            .alloc_zeros::<u8>(len)
            .map_err(|e| e.to_string())?;
        let function = self
            .module
            .load_function("pairs")
            .map_err(|e| e.to_string())?;
        let config = LaunchConfig {
            grid_dim: (u32::try_from(count).map_err(|_| "grid overflow")?, 1, 1),
            block_dim: (128, 1, 1),
            shared_mem_bytes: 0,
        };
        let mut args = self.stream.launch_builder(&function);
        args.arg(&self.vertices)
            .arg(&self.count)
            .arg(&first)
            .arg(&mut output);
        // SAFETY: generated pairs signature is (i64*, u32, u64, u8*).
        // Input has exactly count*(D+1)*D validated coordinates, safe to cast
        // to the precision-selected lane. All grid blocks denote valid tiles;
        // the kernel checks vertex indices before reads, writes exactly
        // count*256 bytes, and all 128 threads reach its shared-memory barrier.
        // Buffers remain alive on this stream through the synchronizing copy.
        unsafe { args.launch(config) }.map_err(|e| e.to_string())?;
        self.stream.synchronize().map_err(|e| e.to_string())?;
        let mut host = vec![0_u8; len];
        self.stream
            .memcpy_dtoh(&output, &mut host)
            .map_err(|e| e.to_string())?;
        self.stream.synchronize().map_err(|e| e.to_string())?;
        self.stream
            .context()
            .check_err()
            .map_err(|e| e.to_string())?;
        Ok(host)
    }
}

fn triangular(n: u64) -> Option<u64> {
    n.checked_mul(n.checked_add(1)?)?.checked_div(2)
}

pub(crate) fn tile_of(a: u32, b: u32) -> Option<u64> {
    triangular(u64::from(b).checked_div(TILE)?)?.checked_add(u64::from(a).checked_div(TILE)?)
}

fn decode(tile: u64, count: u32) -> Option<(u64, u64)> {
    let (mut lo, mut hi) = (0_u64, u64::from(count).div_ceil(TILE));
    while lo.checked_add(1)? < hi {
        let mid = lo.checked_add(hi)?.checked_div(2)?;
        if triangular(mid)? <= tile {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some((tile.checked_sub(triangular(lo)?)?, lo))
}

/// Completed tile watermark survives failures, allowing duplicate-free CPU
/// fallback even after successful earlier batches have reached a consumer.
pub(crate) fn run(
    points: &[i64],
    dimension: usize,
    simplices: &[u32],
    completed: &mut u64,
    sink: &mut impl FnMut(&[(u32, u32)]) -> bool,
) -> Result<Lane, String> {
    let job = guarded(|| Job::new(points, dimension, simplices))?;
    let mut first = 0_u64;
    while first < job.tiles {
        let count = job
            .tiles
            .checked_sub(first)
            .ok_or("tile subtraction")?
            .min(BATCH_TILES);
        #[cfg(test)]
        let count = if let Some(limit) = FAIL_AFTER.get() {
            if first >= limit {
                return Err("injected CUDA batch failure".to_owned());
            }
            count.min(limit)
        } else {
            count
        };
        let flags = guarded(|| job.batch(first, count))?;
        // Decode the entire successful batch before delivering it. A failure
        // here cannot cause half-delivered batches to be repeated on fallback.
        let mut pairs = Vec::new();
        for (offset, chunk) in flags.chunks_exact(256).enumerate() {
            let tile = first
                .checked_add(u64::try_from(offset).map_err(|_| "tile overflow")?)
                .ok_or("tile overflow")?;
            let (a, b) = decode(tile, job.count).ok_or("tile decoding")?;
            for (index, &flag) in chunk.iter().enumerate() {
                if flag == 0 {
                    continue;
                }
                let index = u64::try_from(index).map_err(|_| "pair index")?;
                let ai = a
                    .checked_mul(TILE)
                    .and_then(|x| x.checked_add(index.checked_div(TILE)?))
                    .ok_or("pair index")?;
                let bi = b
                    .checked_mul(TILE)
                    .and_then(|x| x.checked_add(index.checked_rem(TILE)?))
                    .ok_or("pair index")?;
                let (ai, bi) = (
                    u32::try_from(ai).map_err(|_| "pair index")?,
                    u32::try_from(bi).map_err(|_| "pair index")?,
                );
                if ai >= bi || bi >= job.count {
                    return Err("CUDA output index invariant".to_owned());
                }
                pairs.push((ai, bi));
            }
        }
        if !pairs.is_empty() && !sink(&pairs) {
            return Ok(job.lane);
        }
        first = first.checked_add(count).ok_or("tile advance overflow")?;
        *completed = first;
    }
    Ok(job.lane)
}

pub(crate) fn search(
    points: &[i64],
    dimension: usize,
    simplices: &[u32],
) -> Result<Search, String> {
    let mut pairs = Vec::new();
    let lane = run(points, dimension, simplices, &mut 0, &mut |batch| {
        pairs.extend_from_slice(batch);
        true
    })?;
    pairs.sort_unstable();
    Ok(Search {
        pairs,
        backend: Backend::Cuda(lane),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tile_boundaries() {
        for n in [2_u32, 16, 17, 1024, u32::MAX] {
            let sides = u64::from(n).div_ceil(16);
            for b in [0, sides / 2, sides - 1] {
                for a in [0, b / 2, b] {
                    assert_eq!(decode(triangular(b).unwrap() + a, n), Some((a, b)));
                }
            }
        }
    }

    #[test]
    #[ignore = "requires CUDA: proves real streaming and post-delivery failure fallback"]
    fn real_stream_failure_and_cancellation() {
        let points = [0, 0, 4, 0, 0, 4];
        let simplices = [0, 1, 2].repeat(40);
        let expected = unitrisat::intersect::intersecting_pairs(&points, 2, &simplices).unwrap();
        let mut completed = 0;
        let mut delivered = Vec::new();
        assert_eq!(
            run(&points, 2, &simplices, &mut completed, &mut |pairs| {
                delivered.extend_from_slice(pairs);
                true
            })
            .unwrap(),
            Lane::I32
        );
        delivered.sort_unstable();
        assert_eq!(delivered, expected);
        assert_eq!(completed, 6);

        FAIL_AFTER.set(Some(2));
        let collected = Mutex::new(Vec::new());
        let calls = std::sync::atomic::AtomicUsize::new(0);
        crate::stream_pairs(&points, 2, &simplices, 1, &|pairs| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            collected.lock().unwrap().extend_from_slice(pairs);
            true
        })
        .unwrap();
        let mut actual = collected.into_inner().unwrap();
        actual.sort_unstable();
        assert_eq!(actual, expected);
        assert!(calls.load(std::sync::atomic::Ordering::SeqCst) > 1);
        let mut completed = 0;
        let mut calls = 0;
        run(&points, 2, &simplices, &mut completed, &mut |_| {
            calls += 1;
            false
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(completed, 0);
        FAIL_AFTER.set(None);
    }
}
