//! The only module with `unsafe` code: owned cddlib matrices and LP results,
//! reached through the project shim `shim/ucdd.c`.
//!
//! # Thread safety
//!
//! cddlib may be called from several threads at once, without a lock,
//! because of three facts (audit and measurements in
//! `vendor/PATCHES.json`):
//!
//! * Its process-wide objects (the constants `dd_zero`, `dd_one`, ..., the
//!   solver choices, `dd_debug`, `dd_log` and `dd_statStartTime`) are
//!   written only by `dd_set_global_constants`, which [`ready`] runs exactly
//!   once before any other call. Afterwards they are only read.
//! * Its scratch buffers and random state are `_Thread_local` upstream. The
//!   LP pivot counters, the only other objects cddlib writes, are
//!   `_Thread_local` through our patch
//!   `vendor/patches/0001-thread-local-pivot-counters.patch`.
//! * Each matrix and LP is owned by one wrapper, which is neither `Send` nor
//!   `Sync` (it holds a raw pointer), so only one thread touches it.
//!
//! The type system enforces the initialization: every wrapper borrows the
//! [`Ready`] token, which only [`ready`] makes.

use crate::CddError;
use gmp_mpfr_sys::gmp::mpq_t;
use rug::Rational;
use std::{
    ffi::{c_int, c_long},
    marker::PhantomData,
    ptr::NonNull,
    sync::Once,
};

#[repr(C)]
struct CddMatrix {
    _private: [u8; 0],
}

#[repr(C)]
struct CddLp {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn ucdd_init();
    fn ucdd_matrix_new(rows: c_long, cols: c_long, representation: c_int) -> *mut CddMatrix;
    fn ucdd_matrix_free(m: *mut CddMatrix);
    fn ucdd_matrix_rows(m: *mut CddMatrix) -> c_long;
    fn ucdd_matrix_cols(m: *mut CddMatrix) -> c_long;
    fn ucdd_matrix_set(m: *mut CddMatrix, row: c_long, col: c_long, value: *const mpq_t);
    fn ucdd_matrix_get(m: *mut CddMatrix, row: c_long, col: c_long, out: *mut mpq_t);
    fn ucdd_matrix_set_linearity(m: *mut CddMatrix, row: c_long);
    fn ucdd_matrix_is_linearity(m: *mut CddMatrix, row: c_long) -> c_int;
    fn ucdd_matrix_set_objective(m: *mut CddMatrix, sense: c_int);
    fn ucdd_matrix_set_objective_entry(m: *mut CddMatrix, col: c_long, value: *const mpq_t);
    fn ucdd_inequalities(generators: *mut CddMatrix, err: *mut c_int) -> *mut CddMatrix;
    fn ucdd_lp_solve(m: *mut CddMatrix, err: *mut c_int) -> *mut CddLp;
    fn ucdd_lp_free(lp: *mut CddLp);
    fn ucdd_lp_status(lp: *mut CddLp) -> c_int;
    fn ucdd_lp_cols(lp: *mut CddLp) -> c_long;
    fn ucdd_lp_value(lp: *mut CddLp, out: *mut mpq_t);
    fn ucdd_lp_solution(lp: *mut CddLp, col: c_long, out: *mut mpq_t);
}

#[cfg(test)]
unsafe extern "C" {
    fn ucdd_gmp_mpq_set() -> Option<unsafe extern "C" fn(*mut mpq_t, *const mpq_t)>;
    fn ucdd_gmp_header_version() -> c_int;
}

/// The address of the `mpq_set` that the C code calls, and the version of
/// the `gmp.h` it was compiled with (for the shared-GMP test).
#[cfg(test)]
pub(crate) fn c_gmp() -> (usize, c_int) {
    // SAFETY: both functions only return constants; neither touches cddlib.
    let (mpq_set, version) = unsafe { (ucdd_gmp_mpq_set(), ucdd_gmp_header_version()) };
    (mpq_set.map_or(0, |f| f as usize), version)
}

/// cddlib's `dd_NoError`.
const NO_ERROR: c_int = 17;

/// Proof that cddlib's global constants are set; only [`ready`] makes one.
pub(crate) struct Ready {
    _private: (),
}

static INIT: Once = Once::new();

/// Sets cddlib's global constants on the first call, from whichever thread
/// comes first, and returns the proof.
pub(crate) fn ready() -> Ready {
    INIT.call_once(|| {
        // SAFETY: `Once` runs this exactly once, and every other cddlib call
        // needs a `Ready`, which exists only after this returned. `Once`
        // makes its writes visible to every thread that gets a `Ready`.
        unsafe { ucdd_init() }
    });
    Ready { _private: () }
}

/// The error for a call that returned nothing: cddlib's code, or
/// [`CddError::NoResult`] if cddlib reported none.
fn failure(err: c_int) -> CddError {
    if err == NO_ERROR {
        CddError::NoResult
    } else {
        CddError::Cdd(err)
    }
}

/// What kind of rows a matrix holds.
#[derive(Clone, Copy)]
pub(crate) enum Representation {
    /// `b + a·x >= 0` (or `= 0` for linearity rows).
    Inequalities = 1,
    /// `[1, point]` or `[0, ray]`.
    Generators = 2,
}

/// An owned cddlib matrix with checked zero-based indices. Neither `Send`
/// nor `Sync` (raw pointer).
pub(crate) struct Matrix<'r> {
    ptr: NonNull<CddMatrix>,
    rows: c_long,
    cols: c_long,
    _ready: PhantomData<&'r Ready>,
}

impl<'r> Matrix<'r> {
    /// A `rows x cols` matrix of zeros; `None` if cddlib cannot allocate it.
    pub(crate) fn new(
        _ready: &'r Ready,
        rows: c_long,
        cols: c_long,
        representation: Representation,
    ) -> Option<Self> {
        if rows < 0 || cols < 1 {
            return None;
        }
        // SAFETY: cddlib is initialized (`Ready`); the sizes are valid; null
        // is handled.
        let ptr = NonNull::new(unsafe { ucdd_matrix_new(rows, cols, representation as c_int) })?;
        Some(Self {
            ptr,
            rows,
            cols,
            _ready: PhantomData,
        })
    }

    /// Takes ownership of a matrix cddlib returned; `None` for null.
    fn from_raw(ptr: *mut CddMatrix) -> Option<Self> {
        let ptr = NonNull::new(ptr)?;
        // SAFETY: cddlib is initialized (`'r`); `ptr` is a live matrix that
        // cddlib just returned and nobody else owns.
        let (rows, cols) = unsafe {
            (
                ucdd_matrix_rows(ptr.as_ptr()),
                ucdd_matrix_cols(ptr.as_ptr()),
            )
        };
        Some(Self {
            ptr,
            rows,
            cols,
            _ready: PhantomData,
        })
    }

    pub(crate) fn rows(&self) -> c_long {
        self.rows
    }

    pub(crate) fn cols(&self) -> c_long {
        self.cols
    }

    fn contains(&self, row: c_long, col: c_long) -> bool {
        (0..self.rows).contains(&row) && (0..self.cols).contains(&col)
    }

    /// Sets an entry; false if the index is out of range.
    pub(crate) fn set(&mut self, row: c_long, col: c_long, value: &Rational) -> bool {
        if !self.contains(row, col) {
            return false;
        }
        // SAFETY: cddlib is initialized; we own the live matrix; indices
        // checked; `value` is a live mpq_t that cddlib only reads (mpq_set
        // copies it).
        unsafe { ucdd_matrix_set(self.ptr.as_ptr(), row, col, value.as_raw()) };
        true
    }

    /// Reads an entry; `None` if the index is out of range.
    pub(crate) fn get(&self, row: c_long, col: c_long) -> Option<Rational> {
        if !self.contains(row, col) {
            return None;
        }
        let mut out = Rational::new();
        // SAFETY: cddlib is initialized; we own the live matrix; indices
        // checked; `out` is a live, initialized mpq_t that mpq_set
        // overwrites in canonical form.
        unsafe { ucdd_matrix_get(self.ptr.as_ptr(), row, col, out.as_raw_mut()) };
        Some(out)
    }

    /// Marks a row as an equation (inequality matrices) or as generating a
    /// line (generator matrices); false if out of range.
    pub(crate) fn set_linearity(&mut self, row: c_long) -> bool {
        if !(0..self.rows).contains(&row) {
            return false;
        }
        // SAFETY: cddlib is initialized; we own the live matrix; row checked.
        unsafe { ucdd_matrix_set_linearity(self.ptr.as_ptr(), row) };
        true
    }

    pub(crate) fn is_linearity(&self, row: c_long) -> bool {
        // SAFETY: cddlib is initialized; we own the live matrix; row checked
        // first.
        (0..self.rows).contains(&row)
            && unsafe { ucdd_matrix_is_linearity(self.ptr.as_ptr(), row) } != 0
    }

    /// Sets the LP objective: sense 1 maximizes, 2 minimizes.
    pub(crate) fn set_objective(&mut self, sense: c_int, row: &[Rational]) -> bool {
        if c_long::try_from(row.len()).ok() != Some(self.cols) {
            return false;
        }
        // SAFETY: cddlib is initialized; we own the live matrix.
        unsafe { ucdd_matrix_set_objective(self.ptr.as_ptr(), sense) };
        for (col, value) in (0..self.cols).zip(row) {
            // SAFETY: cddlib is initialized; we own the live matrix;
            // col < cols; the objective row has `cols` entries; `value` is
            // only read.
            unsafe { ucdd_matrix_set_objective_entry(self.ptr.as_ptr(), col, value.as_raw()) };
        }
        true
    }

    /// The inequality description of the cone or polytope generated by this
    /// generator matrix.
    pub(crate) fn inequalities(&mut self) -> Result<Matrix<'r>, CddError> {
        let mut err: c_int = NO_ERROR;
        // SAFETY: cddlib is initialized; we own the live generator matrix;
        // `err` is writable.
        let raw = unsafe { ucdd_inequalities(self.ptr.as_ptr(), &raw mut err) };
        // Own the result before looking at `err`, so it is freed either way.
        match Matrix::from_raw(raw) {
            Some(matrix) if err == NO_ERROR => Ok(matrix),
            _ => Err(failure(err)),
        }
    }

    /// Solves the LP of this inequality matrix and its objective.
    pub(crate) fn solve_lp(&mut self) -> Result<Lp<'r>, CddError> {
        let mut err: c_int = NO_ERROR;
        // SAFETY: cddlib is initialized; we own the live inequality matrix;
        // `err` is writable.
        let raw = unsafe { ucdd_lp_solve(self.ptr.as_ptr(), &raw mut err) };
        // Own the result before looking at `err`, so it is freed either way.
        let lp = NonNull::new(raw).map(|ptr| Lp {
            ptr,
            _ready: PhantomData,
        });
        match lp {
            Some(lp) if err == NO_ERROR => Ok(lp),
            _ => Err(failure(err)),
        }
    }
}

impl Drop for Matrix<'_> {
    fn drop(&mut self) {
        // SAFETY: cddlib is initialized; unique owner of a live matrix.
        unsafe { ucdd_matrix_free(self.ptr.as_ptr()) }
    }
}

/// An owned, solved cddlib LP. Neither `Send` nor `Sync` (raw pointer).
pub(crate) struct Lp<'r> {
    ptr: NonNull<CddLp>,
    _ready: PhantomData<&'r Ready>,
}

impl Lp<'_> {
    /// cddlib's `dd_LPStatusType` value.
    pub(crate) fn status(&self) -> c_int {
        // SAFETY: cddlib is initialized; we own the live LP.
        unsafe { ucdd_lp_status(self.ptr.as_ptr()) }
    }

    pub(crate) fn value(&self) -> Rational {
        let mut out = Rational::new();
        // SAFETY: cddlib is initialized; we own the live LP; `out` is a live
        // mpq_t.
        unsafe { ucdd_lp_value(self.ptr.as_ptr(), out.as_raw_mut()) };
        out
    }

    /// The primal solution, including the homogenizing coordinate 0.
    pub(crate) fn solution(&self) -> Vec<Rational> {
        // SAFETY: cddlib is initialized; we own the live LP.
        let cols = unsafe { ucdd_lp_cols(self.ptr.as_ptr()) };
        (0..cols.max(0))
            .map(|col| {
                let mut out = Rational::new();
                // SAFETY: cddlib is initialized; we own the live LP;
                // 0 <= col < d, the length of `sol`; `out` is a live mpq_t.
                unsafe { ucdd_lp_solution(self.ptr.as_ptr(), col, out.as_raw_mut()) };
                out
            })
            .collect()
    }
}

impl Drop for Lp<'_> {
    fn drop(&mut self) {
        // SAFETY: cddlib is initialized; unique owner of a live LP.
        unsafe { ucdd_lp_free(self.ptr.as_ptr()) }
    }
}
