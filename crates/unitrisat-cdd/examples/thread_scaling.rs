//! Thread scaling of the two cddlib workloads UniTriSat runs (task P1.9):
//! one hull per polytope, and one regularity LP per triangulation, built as
//! Julia's `is_regular` builds it. Each workload runs at 1, 2, 4, 8 and 16
//! threads; the speedup shows whether cddlib calls run in parallel.
//!
//! ```text
//! cargo run --release -p unitrisat-cdd --example thread_scaling -- data <data dir> <records per file> <reps>
//! cargo run --release -p unitrisat-cdd --example thread_scaling -- fixtures <reps>
//! ```
//!
//! `data` reads the first records of `3-polytopes/v20.txt`,
//! `4-polytopes/v12.txt` and `5-polytopes/v10.txt` under `<data dir>`
//! (`Polytopes/small-lattice-polytopes/data`); `fixtures` uses `[0,2]^3`,
//! `[0,1]^4`, `3Δ_3` and `2Δ_4`. The triangulations come from lifting all
//! lattice points to a perturbed paraboloid, so they are regular and use
//! every point, and every LP must be feasible.

#![allow(clippy::disallowed_macros, reason = "a measurement tool may assert")]

use rug::{Integer, Rational};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use unitrisat_cdd::{Lp, LpStatus, Sense, hull_inequalities, solve_lp};

fn parse(line: &str) -> Vec<Vec<i64>> {
    line.trim()
        .trim_start_matches("[[")
        .trim_end_matches("]]")
        .split("],[")
        .map(|r| r.split(',').map(|x| x.trim().parse().unwrap()).collect())
        .collect()
}

fn eval(row: &[Rational], p: &[i64]) -> Rational {
    let mut s = row[0].clone();
    for (a, &x) in row[1..].iter().zip(p) {
        s += Rational::from(a * Integer::from(x));
    }
    s
}

fn lattice_points(verts: &[Vec<i64>]) -> Vec<Vec<i64>> {
    let d = verts[0].len();
    let flat: Vec<i64> = verts.iter().flatten().copied().collect();
    let h = hull_inequalities(&flat, d).unwrap();
    let lo: Vec<i64> = (0..d)
        .map(|i| verts.iter().map(|v| v[i]).min().unwrap())
        .collect();
    let hi: Vec<i64> = (0..d)
        .map(|i| verts.iter().map(|v| v[i]).max().unwrap())
        .collect();
    let mut out = Vec::new();
    let mut p = lo.clone();
    loop {
        if h.inequalities.iter().all(|r| eval(r, &p) >= 0)
            && h.equations.iter().all(|r| eval(r, &p) == 0)
        {
            out.push(p.clone());
        }
        let mut i = 0;
        loop {
            if i == d {
                return out;
            }
            if p[i] < hi[i] {
                p[i] += 1;
                break;
            }
            p[i] = lo[i];
            i += 1;
        }
    }
}

fn det(mut m: Vec<Vec<Integer>>) -> Integer {
    let n = m.len();
    let mut sign = 1;
    let mut prev = Integer::from(1);
    for k in 0..n {
        if m[k][k] == 0 {
            match (k + 1..n).find(|&r| m[r][k] != 0) {
                Some(r) => {
                    m.swap(k, r);
                    sign = -sign;
                }
                None => return Integer::new(),
            }
        }
        for i in k + 1..n {
            for j in k + 1..n {
                let v = Integer::from(&m[i][j] * &m[k][k]) - Integer::from(&m[i][k] * &m[k][j]);
                m[i][j] = v / &prev;
            }
        }
        prev = m[k][k].clone();
    }
    m[n - 1][n - 1].clone() * sign
}

/// A regular triangulation by lifting with pseudo-random heights, and its
/// regularity LP rows (Julia's `is_regular`), or None if the lift is not generic.
fn regularity_rows(points: &[Vec<i64>], seed: u64) -> Option<Vec<Vec<Rational>>> {
    let d = points[0].len();
    let n = points.len();
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    // A perturbed paraboloid: every lattice point is on the lower hull, so the
    // triangulation uses all points, as UniTriSat's do.
    let heights: Vec<i64> = points
        .iter()
        .map(|p| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let norm: i64 = p.iter().map(|x| x * x).sum();
            norm * 1_000_000 + ((state >> 33) % 1000) as i64
        })
        .collect();
    let lifted: Vec<Vec<i64>> = points
        .iter()
        .zip(&heights)
        .map(|(p, &h)| p.iter().copied().chain([h]).collect())
        .collect();
    let flat: Vec<i64> = lifted.iter().flatten().copied().collect();
    let hull = hull_inequalities(&flat, d + 1).unwrap();
    let mut simplices = Vec::new();
    for row in &hull.inequalities {
        if row[d + 1] <= 0 {
            continue;
        }
        let on: Vec<usize> = (0..n).filter(|&i| eval(row, &lifted[i]) == 0).collect();
        if on.len() != d + 1 {
            return None;
        }
        simplices.push(on);
    }
    let mut walls: BTreeMap<Vec<usize>, Vec<usize>> = BTreeMap::new();
    for (s, simplex) in simplices.iter().enumerate() {
        for skip in 0..=d {
            let face: Vec<usize> = simplex
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != skip)
                .map(|(_, &v)| v)
                .collect();
            walls.entry(face).or_default().push(s);
        }
    }
    let col = |i: usize| -> Vec<Integer> {
        points[i]
            .iter()
            .map(|&x| Integer::from(x))
            .chain([Integer::from(1)])
            .collect()
    };
    let matrix = |idx: &[usize]| -> Vec<Vec<Integer>> {
        let cols: Vec<Vec<Integer>> = idx.iter().map(|&i| col(i)).collect();
        (0..=d)
            .map(|r| cols.iter().map(|c| c[r].clone()).collect())
            .collect()
    };
    let mut rows = BTreeSet::new();
    for (face, nb) in &walls {
        if nb.len() != 2 {
            continue;
        }
        let s1 = &simplices[nb[0]];
        let q = *simplices[nb[1]].iter().find(|v| !face.contains(v)).unwrap();
        let d1 = det(matrix(s1));
        let sign = if d1 > 0 { 1 } else { -1 };
        let circuit: Vec<usize> = s1.iter().copied().chain([q]).collect();
        let mut coeffs = vec![Integer::new(); n];
        for i in 0..d + 2 {
            let idx: Vec<usize> = circuit
                .iter()
                .enumerate()
                .filter(|&(t, _)| t != i)
                .map(|(_, &v)| v)
                .collect();
            let t = det(matrix(&idx));
            // Julia's 1-based isodd(i) is 0-based even i.
            let s = if i % 2 == 0 { -sign } else { sign };
            coeffs[circuit[i]] += t * s;
        }
        let g = coeffs.iter().fold(Integer::new(), |a, x| a.gcd(x));
        if g != 0 {
            for c in &mut coeffs {
                *c /= &g;
            }
        }
        rows.insert(coeffs);
    }
    Some(
        rows.into_iter()
            .map(|r| {
                std::iter::once(Rational::from(-1))
                    .chain(r.into_iter().map(Rational::from))
                    .collect()
            })
            .collect(),
    )
}

fn throughput(tasks: usize, threads: usize, work: &(dyn Fn(usize) + Sync)) -> Duration {
    let next = AtomicUsize::new(0);
    let start = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= tasks {
                        break;
                    }
                    work(i);
                }
            });
        }
    });
    start.elapsed()
}

fn report(name: &str, tasks: usize, work: &(dyn Fn(usize) + Sync)) {
    throughput(tasks.min(64), 4, work); // warm-up
    let base = throughput(tasks, 1, work);
    println!(
        "{name}: {tasks} calls, 1 thread {:.3} s ({:.1} us/call)",
        base.as_secs_f64(),
        base.as_secs_f64() * 1e6 / tasks as f64
    );
    for t in [2, 4, 8, 16] {
        let el = throughput(tasks, t, work);
        println!(
            "  {t:>2} threads {:.3} s  speedup {:.2}x",
            el.as_secs_f64(),
            base.as_secs_f64() / el.as_secs_f64()
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut polys: Vec<Vec<Vec<i64>>> = Vec::new();
    let reps: usize;
    if args.get(1).map(String::as_str) == Some("data") && args.len() == 5 {
        let dir = &args[2];
        let per_file: usize = args[3].parse().unwrap();
        reps = args[4].parse().unwrap();
        let files = [
            "3-polytopes/v20.txt",
            "4-polytopes/v12.txt",
            "5-polytopes/v10.txt",
        ];
        for f in files {
            let text = std::fs::read_to_string(format!("{dir}/{f}")).unwrap();
            polys.extend(
                text.lines()
                    .filter(|l| !l.trim().is_empty())
                    .take(per_file)
                    .map(parse),
            );
        }
    } else if args.get(1).map(String::as_str) == Some("fixtures") && args.len() == 3 {
        reps = args[2].parse().unwrap();
        let cube = |d: usize, k: i64| -> Vec<Vec<i64>> {
            (0..1usize << d)
                .map(|m| {
                    (0..d)
                        .map(|i| if m >> i & 1 == 1 { k } else { 0 })
                        .collect()
                })
                .collect()
        };
        let simplex = |d: usize, k: i64| -> Vec<Vec<i64>> {
            (0..=d)
                .map(|j| (0..d).map(|i| if j == i + 1 { k } else { 0 }).collect())
                .collect()
        };
        polys = vec![cube(3, 2), cube(4, 1), simplex(3, 3), simplex(4, 2)];
    } else {
        eprintln!("usage: thread_scaling data <dir> <records per file> <reps> | fixtures <reps>");
        std::process::exit(2);
    }
    let hulls: Vec<(Vec<i64>, usize)> = polys
        .iter()
        .map(|p| (p.iter().flatten().copied().collect(), p[0].len()))
        .collect();
    let mut lps = Vec::new();
    let mut sizes = Vec::new();
    for (k, p) in polys.iter().enumerate() {
        let pts = lattice_points(p);
        if let Some(rows) = (0..20).find_map(|s| regularity_rows(&pts, (k * 100 + s) as u64))
            && !rows.is_empty()
        {
            sizes.push((rows.len(), pts.len()));
            lps.push(rows);
        }
    }
    let (r, c): (usize, usize) = sizes.iter().fold((0, 0), |(a, b), &(x, y)| (a + x, b + y));
    println!(
        "{} polytopes, {} regularity LPs (mean {:.1} rows x {:.1} variables, max rows {})",
        polys.len(),
        lps.len(),
        r as f64 / lps.len() as f64,
        c as f64 / lps.len() as f64,
        sizes.iter().map(|s| s.0).max().unwrap()
    );
    let objectives: Vec<Vec<Rational>> = lps
        .iter()
        .map(|rows| vec![Rational::new(); rows[0].len()])
        .collect();
    // Check once: every lifted triangulation is regular, so every LP is feasible.
    for (rows, obj) in lps.iter().zip(&objectives) {
        let s = solve_lp(&Lp {
            inequalities: rows,
            equations: &[],
            objective: obj,
            sense: Sense::Maximize,
        })
        .unwrap();
        assert_eq!(s.status, LpStatus::Optimal);
    }
    report("hull", hulls.len() * reps, &|i| {
        let (pts, d) = &hulls[i % hulls.len()];
        hull_inequalities(pts, *d).unwrap();
    });
    report("regularity LP", lps.len() * reps, &|i| {
        let k = i % lps.len();
        let s = solve_lp(&Lp {
            inequalities: &lps[k],
            equations: &[],
            objective: &objectives[k],
            sense: Sense::Maximize,
        })
        .unwrap();
        assert_eq!(s.status, LpStatus::Optimal);
    });
}
