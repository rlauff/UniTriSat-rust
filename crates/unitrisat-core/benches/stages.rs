//! Reproducible Criterion stages on preserved P0.3 Julia fixtures.
//!
//! Run `RAYON_NUM_THREADS=1 cargo bench -p unitrisat --bench stages` (also 16).
//! `-- --test` performs output checks and one invocation per benchmark.
//! Fixture loading, prior pipeline stages, and classification triangulations
//! are outside each timed closure. Errors abort, never become fast samples.

use criterion::{BenchmarkId, Criterion};
use std::{collections::BTreeMap, error::Error, fs, hint::black_box, path::Path, time::Duration};
use unitrisat_core::{
    cnf::{self, Cnf},
    enumerate::{self, AnyTriangulation, Enumeration, Goal, Keep, Options, Outcome},
    flag::Flag,
    hull::{self, Facet},
    intersect,
    lattice::{self, Central, Faces, Restriction},
    matrix::Matrix,
    regular::Regularity,
    solve::Backend,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Case {
    name: &'static str,
    vertices: Matrix,
    facets: Vec<Facet>,
    points: Matrix,
    simplices: Vec<u32>,
    faces: Faces,
    central: Central,
    pairs: Vec<(u32, u32)>,
    full: Cnf,
    reduced: Cnf,
    triangulations: Vec<Vec<u32>>,
    expected_count: Option<u64>,
    expected_regular: Option<u64>,
}

fn rows<T: std::str::FromStr>(path: &Path) -> Result<Vec<Vec<T>>>
where
    T::Err: Error + 'static,
{
    fs::read_to_string(path)?
        .lines()
        .map(|line| {
            line.split('\t')
                .map(|s| s.parse().map_err(Into::into))
                .collect()
        })
        .collect()
}

fn matrix(path: &Path) -> Result<Matrix> {
    let rows = rows::<i64>(path)?;
    let width = rows.first().ok_or("empty matrix fixture")?.len();
    if rows.iter().any(|r| r.len() != width) {
        return Err("ragged fixture".into());
    }
    Ok(Matrix::new(width, rows.into_iter().flatten().collect())?)
}

fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}

fn options(backend: Backend, keep: Keep) -> Options {
    Options {
        backend,
        goal: Goal::All,
        keep,
        max_solutions: None,
        deadline: None,
    }
}

fn verify_enumeration(result: &Enumeration, count: u64) -> Result<()> {
    ensure(
        result.outcome == Outcome::Complete && result.found == count,
        "enumeration differs from Julia fixture",
    )
}

impl Case {
    fn load(name: &'static str, classify: bool) -> Result<Self> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("benches/fixtures")
            .join(name);
        let meta: BTreeMap<String, String> = rows::<String>(&dir.join("meta.tsv"))?
            .into_iter()
            .map(|row| match row.as_slice() {
                [key, value] => Ok((key.clone(), value.clone())),
                _ => Err("invalid metadata".into()),
            })
            .collect::<Result<_>>()?;
        let number =
            |key: &str| -> Result<u64> { Ok(meta.get(key).ok_or("missing metadata")?.parse()?) };
        ensure(
            meta.get("restriction").map(String::as_str) == Some("unimodular"),
            "fixture restriction",
        )?;
        let vertices = matrix(&dir.join("vertices.tsv"))?;
        ensure(
            lattice::project_to_full_dimension(&vertices)?.is_none(),
            "baseline unexpectedly projects",
        )?;
        let facets = hull::facets(&vertices)?;
        let points = lattice::lattice_points(&vertices, &facets)?;
        ensure(
            points == matrix(&dir.join("points.tsv"))?,
            "lattice points differ",
        )?;
        let simplices = lattice::simplices(&points, Restriction::Unimodular)?;
        let expected_simplices: Vec<u32> = rows::<u32>(&dir.join("simplices.tsv"))?
            .into_iter()
            .flatten()
            .collect();
        ensure(simplices == expected_simplices, "simplices differ")?;
        let width = points.columns() + 1;
        let count = simplices.len() / width;
        let faces = lattice::faces(&points, &facets, &simplices)?;
        let central = lattice::central(&points, &simplices, &faces)?;
        let pairs = intersect::intersecting_pairs(points.values(), points.columns(), &simplices)?;
        let expected_pairs: Vec<(u32, u32)> = rows::<u32>(&dir.join("pairs.tsv"))?
            .into_iter()
            .map(|row| match row.as_slice() {
                [a, b] => Ok((*a, *b)),
                _ => Err("invalid pair fixture".into()),
            })
            .collect::<Result<_>>()?;
        ensure(pairs == expected_pairs, "pairs differ")?;
        let full = cnf::formula(count, &pairs, &faces)?;
        let reduced = cnf::reduced(count, &faces, &central)?;
        let mut expected_clauses = vec![(1..=i32::try_from(count)?).collect::<Vec<_>>()];
        for &(a, b) in &expected_pairs {
            expected_clauses.push(vec![-(i32::try_from(a)? + 1), -(i32::try_from(b)? + 1)]);
        }
        // Julia emits one row per internal simplex face, including empty covers.
        for line in fs::read_to_string(dir.join("face_clauses.tsv"))?.lines() {
            let (simplex, covers) = line.split_once('\t').ok_or("invalid face clause fixture")?;
            let mut clause = vec![-(simplex.parse::<i32>()? + 1)];
            for label in covers.split_whitespace() {
                clause.push(label.parse::<i32>()? + 1);
            }
            expected_clauses.push(clause);
        }
        let mut actual_clauses: Vec<Vec<i32>> = full.iter().map(<[i32]>::to_vec).collect();
        expected_clauses.sort_unstable();
        actual_clauses.sort_unstable();
        ensure(
            expected_clauses == actual_clauses,
            "CNF/face clauses differ",
        )?;
        ensure(
            points.rows() as u64 == number("points")?
                && count as u64 == number("simplices")?
                && pairs.len() as u64 == number("intersecting_pairs")?,
            "metadata counts differ",
        )?;
        let mut triangulations = Vec::new();
        let expected_count = if classify {
            let count = number("triangulations")?;
            let enumeration = enumerate::serial(
                &full,
                &options(Backend::PicoSat, Keep::All),
                &AnyTriangulation,
            )?;
            verify_enumeration(&enumeration, count)?;
            triangulations = enumeration.triangulations;
            let regular = Regularity::new(&points, &simplices, &faces)?;
            let flag = Flag::new(&simplices, width, points.rows())?;
            let regular_count = triangulations.iter().try_fold(0_u64, |n, t| {
                regular.is_regular(t).map(|v| n + u64::from(v))
            })?;
            let flag_count = triangulations
                .iter()
                .try_fold(0_u64, |n, t| flag.is_flag(t).map(|v| n + u64::from(v)))?;
            ensure(
                regular_count == number("regular_triangulations")?,
                "regular count differs",
            )?;
            let flag_text = fs::read_to_string(dir.join("flag_reference.tsv"))?;
            let flag_row: Vec<_> = flag_text
                .lines()
                .nth(1)
                .ok_or("missing flag reference")?
                .split('\t')
                .collect();
            ensure(
                flag_row
                    .get(8)
                    .ok_or("missing flag count")?
                    .parse::<u64>()?
                    == flag_count,
                "flag count differs",
            )?;
            for backend in [Backend::PicoSat, Backend::Cadical] {
                let opt = options(backend, Keep::None);
                verify_enumeration(&enumerate::serial(&full, &opt, &AnyTriangulation)?, count)?;
                verify_enumeration(
                    &enumerate::parallel_split(
                        &full,
                        central.simplices(),
                        &opt,
                        &AnyTriangulation,
                    )?,
                    count,
                )?;
                verify_enumeration(
                    &enumerate::incremental(
                        &reduced,
                        &points,
                        &simplices,
                        &opt,
                        &AnyTriangulation,
                    )?,
                    count,
                )?;
            }
            Some(count)
        } else {
            None
        };
        Ok(Self {
            name,
            vertices,
            facets,
            points,
            simplices,
            faces,
            central,
            pairs,
            full,
            reduced,
            triangulations,
            expected_count,
            expected_regular: if classify {
                Some(number("regular_triangulations")?)
            } else {
                None
            },
        })
    }

    fn bench(&self, criterion: &mut Criterion) -> Result<()> {
        let mut group = criterion.benchmark_group(self.name);
        macro_rules! stage {
            ($name:expr, $expr:expr) => {
                group.bench_function($name, |b| {
                    b.iter(|| black_box($expr.expect("benchmark stage failed")))
                });
            };
        }
        stage!(
            "projection",
            lattice::project_to_full_dimension(black_box(&self.vertices))
        );
        stage!("hull", hull::facets(black_box(&self.vertices)));
        stage!(
            "scan",
            lattice::lattice_points(black_box(&self.vertices), black_box(&self.facets))
        );
        stage!(
            "hull_scan",
            (|| -> Result<Matrix> {
                let facets = hull::facets(black_box(&self.vertices))?;
                Ok(lattice::lattice_points(&self.vertices, &facets)?)
            })()
        );
        stage!(
            "simplices",
            lattice::simplices(black_box(&self.points), Restriction::Unimodular)
        );
        stage!(
            "faces",
            lattice::faces(black_box(&self.points), &self.facets, &self.simplices)
        );
        stage!(
            "central",
            lattice::central(black_box(&self.points), &self.simplices, &self.faces)
        );
        stage!(
            "intersections",
            intersect::intersecting_pairs(
                black_box(self.points.values()),
                self.points.columns(),
                &self.simplices
            )
        );
        let count = self.simplices.len() / (self.points.columns() + 1);
        stage!(
            "cnf_full",
            cnf::formula(count, black_box(&self.pairs), &self.faces)
        );
        stage!(
            "cnf_reduced",
            cnf::reduced(count, black_box(&self.faces), &self.central)
        );
        if self.expected_count.is_some() {
            for (label, backend) in [("picosat", Backend::PicoSat), ("cadical", Backend::Cadical)] {
                let opt = options(backend, Keep::None);
                stage!(
                    BenchmarkId::new("enumerate_serial", label),
                    enumerate::serial(black_box(&self.full), &opt, &AnyTriangulation)
                );
                stage!(
                    BenchmarkId::new("enumerate_split", label),
                    enumerate::parallel_split(
                        black_box(&self.full),
                        self.central.simplices(),
                        &opt,
                        &AnyTriangulation
                    )
                );
                // Incremental includes streamed intersection work by contract.
                stage!(
                    BenchmarkId::new("enumerate_incremental", label),
                    enumerate::incremental(
                        black_box(&self.reduced),
                        &self.points,
                        &self.simplices,
                        &opt,
                        &AnyTriangulation
                    )
                );
            }
            let end_to_end = pipeline_regular(&self.vertices)?;
            verify_enumeration(&end_to_end, self.expected_count.ok_or("missing count")?)?;
            ensure(
                end_to_end.targets == self.expected_regular.ok_or("missing regular count")?,
                "end-to-end regular count",
            )?;
            stage!(
                "pipeline_regular_all",
                pipeline_regular(black_box(&self.vertices))
            );
            let regular = Regularity::new(&self.points, &self.simplices, &self.faces)?;
            let flag = Flag::new(
                &self.simplices,
                self.points.columns() + 1,
                self.points.rows(),
            )?;
            stage!(
                "regularity_setup",
                Regularity::new(black_box(&self.points), &self.simplices, &self.faces)
            );
            stage!(
                "flag_setup",
                Flag::new(
                    black_box(&self.simplices),
                    self.points.columns() + 1,
                    self.points.rows()
                )
            );
            stage!(
                "regularity_all",
                self.triangulations.iter().try_fold(0_u64, |n, t| regular
                    .is_regular(black_box(t))
                    .map(|v| n + u64::from(v)))
            );
            stage!(
                "flag_all",
                self.triangulations.iter().try_fold(0_u64, |n, t| flag
                    .is_flag(black_box(t))
                    .map(|v| n + u64::from(v)))
            );
        }
        group.finish();
        Ok(())
    }
}

fn pipeline_regular(vertices: &Matrix) -> Result<Enumeration> {
    let projected = lattice::project_to_full_dimension(vertices)?;
    let vertices = projected.as_ref().unwrap_or(vertices);
    let facets = hull::facets(vertices)?;
    let points = lattice::lattice_points(vertices, &facets)?;
    let simplices = lattice::simplices(&points, Restriction::Unimodular)?;
    let faces = lattice::faces(&points, &facets, &simplices)?;
    let pairs = intersect::intersecting_pairs(points.values(), points.columns(), &simplices)?;
    let full = cnf::formula(simplices.len() / (points.columns() + 1), &pairs, &faces)?;
    let regular = Regularity::new(&points, &simplices, &faces)?;
    Ok(enumerate::serial(
        &full,
        &options(Backend::PicoSat, Keep::None),
        &regular,
    )?)
}

fn embedded_projection(criterion: &mut Criterion) -> Result<()> {
    // Primitive Z^2 embedding of conv(0, 2e1, 3e2) into Z^4, from
    // lattice/tests.rs; determinant area is independently known to be six.
    let embedded = Matrix::new(4, vec![5, -1, 4, 0, 7, -1, 10, -4, 11, 2, 19, 21])?;
    let projected = lattice::project_to_full_dimension(&embedded)?.ok_or("expected projection")?;
    ensure(
        projected.columns() == 2 && projected.rows() == 3,
        "projection shape",
    )?;
    let rows: Vec<_> = projected.iter_rows().collect();
    ensure(
        rows[0] == [0, 0] && (rows[1][0] * rows[2][1] - rows[1][1] * rows[2][0]).abs() == 6,
        "projection volume",
    )?;
    criterion.bench_function("embedded_triangle/projection", |b| {
        b.iter(|| {
            black_box(
                lattice::project_to_full_dimension(black_box(&embedded))
                    .expect("projection failed"),
            )
        })
    });
    Ok(())
}

fn main() -> Result<()> {
    // Construct and check every fixture before any measurement; no disk I/O,
    // reference parsing, or earlier pipeline stages enter isolated timings.
    let cases = [
        ("cube3", true),
        ("2simplex3", true),
        ("data3", true),
        ("data4", false),
        ("data6", false),
    ]
    .into_iter()
    .map(|(name, solve)| Case::load(name, solve))
    .collect::<Result<Vec<_>>>()?;
    let mut criterion = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_millis(500))
        .configure_from_args();
    for case in &cases {
        case.bench(&mut criterion)?;
    }
    embedded_projection(&mut criterion)?;
    criterion.final_summary();
    Ok(())
}
