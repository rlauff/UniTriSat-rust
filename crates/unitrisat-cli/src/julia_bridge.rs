//! Strict version-one file protocol used by the Julia compatibility package.
//! Mathematical work is delegated to [`job::run_detailed`].

use crate::job::{self, IntersectionBackend, Mode, Settings};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufReader, Write},
    num::NonZeroU64,
    path::{Path, PathBuf},
    time::Instant,
};
use unitrisat::{
    enumerate::{Goal, Keep},
    input,
    lattice::Restriction,
    matrix::Matrix,
    solve::Backend,
};

/// Protocol, input, computation or output failure. No partial batch is successful.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// File access failed.
    #[error("bridge I/O: {0}")]
    Io(#[from] std::io::Error),
    /// JSON was malformed or did not match the strict protocol.
    #[error("bridge JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// A value was invalid.
    #[error("bridge request: {0}")]
    Invalid(String),
    /// A polytope file could not be read.
    #[error(transparent)]
    Input(#[from] input::InputError),
    /// A polytope computation failed.
    #[error(transparent)]
    Job(#[from] job::JobError),
    /// The requested worker pool could not start.
    #[error("bridge workers: {0}")]
    Workers(#[from] rayon::ThreadPoolBuildError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol: u32,
    input: Input,
    settings: RequestSettings,
    threads: usize,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Matrices { matrices: Vec<Vec<Vec<String>>> },
    File { path: PathBuf },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Device {
    Cpu,
    Gpu,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Retention {
    None,
    First,
    All,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Solver {
    Picosat,
    Cadical,
    Kissat,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SearchMode {
    Serial,
    Split,
    Incremental,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestSettings {
    backend: Device,
    unimodular: bool,
    regular: bool,
    flag: bool,
    find_all: bool,
    keep: Retention,
    solver: Solver,
    mode: SearchMode,
    // `deserialize_with` requires these fields to be present, even for null.
    #[serde(deserialize_with = "required_option")]
    max_solutions: Option<String>,
    #[serde(deserialize_with = "required_option")]
    max_regularity_tests: Option<String>,
}

fn required_option<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}

fn invalid(message: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(message.into())
}

fn coordinate(text: &str) -> Result<i64, BridgeError> {
    let value = text
        .parse::<i64>()
        .map_err(|_| invalid("coordinate must fit i64"))?;
    if value.to_string() != text {
        return Err(invalid("coordinate must be a canonical decimal string"));
    }
    Ok(value)
}

fn cap(text: Option<&str>) -> Result<Option<NonZeroU64>, BridgeError> {
    text.map(|text| {
        let value = text
            .parse::<u64>()
            .map_err(|_| invalid("cap must fit u64"))?;
        if value.to_string() != text {
            return Err(invalid("cap must be a canonical decimal string"));
        }
        NonZeroU64::new(value).ok_or_else(|| invalid("cap must be positive or null"))
    })
    .transpose()
}

impl RequestSettings {
    fn settings(&self) -> Result<(Settings, Keep), BridgeError> {
        Ok((
            Settings {
                intersection_backend: match self.backend {
                    Device::Cpu => IntersectionBackend::Cpu,
                    Device::Gpu => IntersectionBackend::Gpu,
                },
                restriction: if self.unimodular {
                    Restriction::Unimodular
                } else {
                    Restriction::Nondegenerate
                },
                mode: match self.mode {
                    SearchMode::Serial => Mode::Serial,
                    SearchMode::Split => Mode::Split,
                    SearchMode::Incremental => Mode::Incremental,
                },
                backend: match self.solver {
                    Solver::Picosat => Backend::PicoSat,
                    Solver::Cadical => Backend::Cadical,
                    Solver::Kissat => Backend::Kissat,
                },
                goal: if self.find_all {
                    Goal::All
                } else {
                    Goal::First
                },
                regular: self.regular,
                regular_learn: false,
                flag: self.flag,
                flag_sat: false,
                max_solutions: cap(self.max_solutions.as_deref())?,
                max_regularity_tests: cap(self.max_regularity_tests.as_deref())?,
                timeout: None,
            },
            match self.keep {
                Retention::None => Keep::None,
                Retention::First => Keep::First,
                Retention::All => Keep::All,
            },
        ))
    }
}

impl Input {
    fn matrices(self) -> Result<Vec<Matrix>, BridgeError> {
        match self {
            Self::Matrices { matrices } => matrices
                .into_iter()
                .map(|rows| {
                    let width = rows.first().map_or(0, Vec::len);
                    if width == 0 || rows.iter().any(|row| row.len() != width) {
                        return Err(invalid("vertices must be a nonempty rectangular matrix"));
                    }
                    let values = rows
                        .into_iter()
                        .flatten()
                        .map(|text| coordinate(&text))
                        .collect::<Result<Vec<_>, _>>()?;
                    Matrix::new(width, values).map_err(|error| invalid(error.to_string()))
                })
                .collect(),
            Self::File { path } => {
                if !path.is_absolute() {
                    return Err(invalid("input file path must be absolute"));
                }
                input::open_points(&path)?
                    .map(|record| {
                        record
                            .map(|record| record.vertices)
                            .map_err(BridgeError::from)
                    })
                    .collect()
            }
        }
    }
}

type Solutions = Vec<Vec<Vec<Vec<String>>>>;

#[derive(Serialize)]
struct Counts {
    triangulations: String,
    regular: String,
    flag: String,
    quadratic: String,
}

#[derive(Serialize)]
struct Step {
    name: &'static str,
    seconds: f64,
}

#[derive(Serialize)]
struct ResultItem {
    counts: Counts,
    status: &'static str,
    seconds: f64,
    solutions: Solutions,
    steps: [Step; 3],
}

#[derive(Serialize)]
struct Response {
    protocol: u32,
    results: Vec<ResultItem>,
    seconds: f64,
}

fn solutions(detailed: &job::Detailed) -> Result<Solutions, BridgeError> {
    let dimension = detailed.points.columns();
    let width = dimension
        .checked_add(1)
        .ok_or_else(|| invalid("simplex width overflow"))?;
    detailed
        .triangulations
        .iter()
        .map(|triangulation| {
            triangulation
                .iter()
                .map(|&simplex| {
                    let start = usize::try_from(simplex)
                        .ok()
                        .and_then(|s| s.checked_mul(width))
                        .ok_or_else(|| invalid("simplex index overflow"))?;
                    let end = start
                        .checked_add(width)
                        .ok_or_else(|| invalid("simplex end overflow"))?;
                    let indices = detailed
                        .simplices
                        .get(start..end)
                        .ok_or_else(|| invalid("retained simplex index out of range"))?;
                    indices
                        .iter()
                        .map(|&point| {
                            let start = usize::try_from(point)
                                .ok()
                                .and_then(|p| p.checked_mul(dimension))
                                .ok_or_else(|| invalid("point index overflow"))?;
                            let end = start
                                .checked_add(dimension)
                                .ok_or_else(|| invalid("point end overflow"))?;
                            let row = detailed
                                .points
                                .values()
                                .get(start..end)
                                .ok_or_else(|| invalid("retained point index out of range"))?;
                            Ok(row.iter().map(ToString::to_string).collect())
                        })
                        .collect()
                })
                .collect()
        })
        .collect()
}

fn result(detailed: job::Detailed) -> Result<ResultItem, BridgeError> {
    let row = detailed.row;
    let retained = solutions(&detailed)?;
    Ok(ResultItem {
        counts: Counts {
            triangulations: row.triangulations.to_string(),
            regular: row.regular.to_string(),
            flag: row.flag.to_string(),
            quadratic: row.quadratic.to_string(),
        },
        status: row.status.as_str(),
        seconds: row.seconds,
        solutions: retained,
        steps: [
            Step {
                name: "Lattice geometry",
                seconds: detailed.timings.lattice,
            },
            Step {
                name: "Formula construction",
                seconds: detailed.timings.formula,
            },
            Step {
                name: "Search and classification",
                seconds: detailed.timings.search,
            },
        ],
    })
}

fn execute(request: Request) -> Result<Response, BridgeError> {
    let start = Instant::now();
    if request.protocol != 1 {
        return Err(invalid("unsupported protocol version"));
    }
    if request.threads == 0 {
        return Err(invalid("threads must be positive"));
    }
    let (settings, keep) = request.settings.settings()?;
    settings
        .validate()
        .map_err(|error| invalid(error.to_string()))?;
    let matrices = request.input.matrices()?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(request.threads)
        .build()?;
    let results = pool.install(|| {
        matrices
            .iter()
            .map(|vertices| result(job::run_detailed(1, vertices, &settings, keep)?))
            .collect::<Result<Vec<_>, BridgeError>>()
    })?;
    Ok(Response {
        protocol: 1,
        results,
        seconds: start.elapsed().as_secs_f64(),
    })
}

/// Reads a complete request and writes one complete response after every job
/// succeeds. The response path must not exist, preventing stale-success reuse.
/// Failure always propagates to a nonzero process exit in the bridge executable.
pub fn execute_file(request_path: &Path, response_path: &Path) -> Result<(), BridgeError> {
    let request: Request = serde_json::from_reader(BufReader::new(File::open(request_path)?))?;
    let response = execute(request)?;
    let bytes = serde_json::to_vec(&response)?;
    let mut output = File::create_new(response_path)?;
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> serde_json::Value {
        json!({"protocol":1,"threads":1,"input":{"kind":"matrices","matrices":[[["0","0"],["1","0"],["0","1"]]]},"settings":{"backend":"cpu","unimodular":true,"regular":false,"flag":false,"find_all":true,"keep":"all","solver":"picosat","mode":"serial","max_solutions":null,"max_regularity_tests":null}})
    }

    #[test]
    fn kissat_bridge_validates_empty_batches_and_missing_input_before_work() {
        for input in [
            json!({"kind":"matrices","matrices":[]}),
            json!({"kind":"file","path":"/missing-kissat-input"}),
        ] {
            for (key, option) in [
                ("mode", json!("incremental")),
                ("find_all", json!(true)),
                ("regular", json!(true)),
                ("flag", json!(true)),
            ] {
                let mut value = request();
                value["input"] = input.clone();
                value["settings"]["solver"] = json!("kissat");
                value["settings"]["find_all"] = json!(false);
                value["settings"][key] = option;
                let error = execute(serde_json::from_value(value).unwrap())
                    .err()
                    .unwrap();
                assert!(matches!(error, BridgeError::Invalid(_)));
                assert!(error.to_string().contains("Kissat"));
            }
        }
    }

    #[test]
    #[cfg(not(windows))]
    fn kissat_bridge_retains_first_unfiltered_triangulation() {
        for mode in ["serial", "split"] {
            let mut value = request();
            value["settings"]["solver"] = json!("kissat");
            value["settings"]["find_all"] = json!(false);
            value["settings"]["mode"] = json!(mode);
            let response = execute(serde_json::from_value(value).unwrap()).unwrap();
            assert_eq!(response.results[0].counts.triangulations, "1");
            assert_eq!(response.results[0].solutions.len(), 1);
        }
    }

    #[test]
    fn integers_and_caps_fail_closed() {
        for text in ["+1", "01", "-0", "1.0", " 1", "9223372036854775808"] {
            assert!(coordinate(text).is_err(), "{text}");
        }
        assert_eq!(coordinate("-9223372036854775808").unwrap(), i64::MIN);
        for text in ["0", "-1", "01", "18446744073709551616"] {
            assert!(cap(Some(text)).is_err(), "{text}");
        }
        assert!(cap(None).unwrap().is_none());
    }

    #[test]
    fn unknown_missing_and_wrong_type_fields_fail_closed() {
        let mut value = request();
        value["extra"] = json!(1);
        assert!(serde_json::from_value::<Request>(value).is_err());
        let mut value = request();
        value["settings"]
            .as_object_mut()
            .unwrap()
            .remove("max_solutions");
        assert!(serde_json::from_value::<Request>(value).is_err());
        let mut value = request();
        value["input"]["matrices"][0][0][0] = json!(0);
        assert!(serde_json::from_value::<Request>(value).is_err());
        let mut value = request();
        value["settings"]["solver"] = json!("d4");
        assert!(serde_json::from_value::<Request>(value).is_err());
    }

    #[test]
    fn exact_coordinates_and_empty_batches() {
        let response = execute(serde_json::from_value(request()).unwrap()).unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].counts.triangulations, "1");
        assert_eq!(response.results[0].solutions.len(), 1);
        let mut vertices = response.results[0].solutions[0][0].clone();
        vertices.sort();
        assert_eq!(
            vertices,
            vec![vec!["0", "0"], vec!["0", "1"], vec!["1", "0"]]
        );
        let mut value = request();
        value["input"]["matrices"] = json!([]);
        assert!(
            execute(serde_json::from_value(value).unwrap())
                .unwrap()
                .results
                .is_empty()
        );
    }

    #[test]
    fn coordinate_transport_preserves_values_beyond_f64_precision() {
        let base = 9_007_199_254_740_993_i64;
        let points = Matrix::new(2, vec![base, 0, base + 1, 0, base, 1]).unwrap();
        let detailed = job::Detailed {
            row: job::Row {
                index: 1,
                status: job::Status::Ok,
                triangulations: 1,
                regular: 0,
                flag: 0,
                quadratic: 0,
                seconds: 0.0,
            },
            outcome: unitrisat::enumerate::Outcome::Complete,
            points,
            simplices: vec![0, 1, 2],
            triangulations: vec![vec![0]],
            timings: job::StageTimes::default(),
        };
        let value = serde_json::to_value(result(detailed).unwrap()).unwrap();
        assert_eq!(value["solutions"][0][0][0][0], "9007199254740993");
        assert_eq!(value["solutions"][0][0][1][0], "9007199254740994");
        assert_eq!(
            coordinate(value["solutions"][0][0][0][0].as_str().unwrap()).unwrap(),
            base
        );
    }

    #[test]
    fn file_input_preserves_order_and_batch_failure_writes_no_success() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("unitrisat-julia-{}-{unique}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let input_path = directory.join("vertices.txt");
        let request_path = directory.join("request.json");
        let response_path = directory.join("response.json");
        std::fs::write(
            &input_path,
            "[[0,0],[1,0],[0,1]]\n[[0,0],[1,0],[0,1],[1,1]]\n",
        )
        .unwrap();
        let mut value = request();
        value["input"] = json!({"kind":"file","path":input_path});
        std::fs::write(&request_path, serde_json::to_vec(&value).unwrap()).unwrap();
        execute_file(&request_path, &response_path).unwrap();
        let response: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&response_path).unwrap()).unwrap();
        assert_eq!(response["results"][0]["counts"]["triangulations"], "1");
        assert_eq!(response["results"][1]["counts"]["triangulations"], "2");
        std::fs::remove_file(&response_path).unwrap();
        // A valid first record followed by an invalid one must not leave an
        // apparently successful prefix response.
        std::fs::write(&input_path, "[[0,0],[1,0],[0,1]]\n[[0,broken]]\n").unwrap();
        assert!(execute_file(&request_path, &response_path).is_err());
        assert!(!response_path.exists());
        // A numerical failure in a later job is also all-or-nothing.
        std::fs::write(&input_path, "[[0,0],[1,0],[0,1]]\n[[0,0],[0,0]]\n").unwrap();
        assert!(execute_file(&request_path, &response_path).is_err());
        assert!(!response_path.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
