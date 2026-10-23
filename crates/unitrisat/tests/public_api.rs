//! Public API integration coverage for arbitrary input matrices.
#![allow(clippy::disallowed_macros, reason = "integration tests assert results")]

use std::num::NonZeroU64;
use unitrisat::{
    Matrix, Settings,
    enumerate::{Goal, Outcome},
    triangulate,
};

#[test]
fn arbitrary_square_returns_geometry_and_obeys_caps() {
    let vertices = Matrix::new(2, vec![0, 0, 1, 0, 0, 1, 1, 1]).unwrap();
    let settings = Settings {
        goal: Goal::All,
        ..Settings::default()
    };
    let result = triangulate(&vertices, &settings).unwrap();
    assert_eq!(result.row.triangulations, 2);
    assert_eq!(result.outcome, Outcome::Complete);
    assert_eq!(result.triangulations.len(), 2);
    assert_eq!(result.points.columns(), 2);
    assert!(result.triangulations.iter().all(|t| t.len() == 2));
    let capped = triangulate(
        &vertices,
        &Settings {
            max_solutions: NonZeroU64::new(1),
            ..settings
        },
    )
    .unwrap();
    assert_eq!(capped.outcome, Outcome::Capped);
    assert_eq!(capped.row.triangulations, 1);
    assert_eq!(capped.triangulations.len(), 1);
}
