//! Errors of the I/O boundary and their exit codes.

use std::path::PathBuf;

use accessmod_core::solver::SolveError;
use accessmod_core::store::StoreError;

/// Error class, mapped to the CLI exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The job or its inputs are invalid (exit code 2).
    Validation,
    /// A file could not be read or written (exit code 3).
    Io,
    /// The computation failed (exit code 4).
    Computation,
    /// The run was cancelled (exit code 130).
    Cancelled,
}

impl ErrorKind {
    /// CLI exit code.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Validation => 2,
            Self::Io => 3,
            Self::Computation => 4,
            Self::Cancelled => 130,
        }
    }
}

/// Error of the I/O boundary.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Invalid job or input data. `issues` lists every problem found, so the
    /// user can fix them in one pass.
    #[error("invalid job: {}", .issues.join("; "))]
    Validation {
        /// Human-readable problems.
        issues: Vec<String>,
    },
    /// GDAL failed on a file.
    #[error("{path}: {source}")]
    Gdal {
        /// File being read or written.
        path: PathBuf,
        /// GDAL error.
        #[source]
        source: gdal::errors::GdalError,
    },
    /// Filesystem failure.
    #[error("{path}: {source}")]
    File {
        /// File or directory.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Scratch storage failure.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Solver failure.
    #[error("computation failed: {0}")]
    Solve(SolveError),
    /// Cancelled by the user.
    #[error("cancelled")]
    Cancelled,
}

impl Error {
    /// Class of this error.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Validation { .. } => ErrorKind::Validation,
            Self::Gdal { .. } | Self::File { .. } | Self::Store(_) => ErrorKind::Io,
            Self::Solve(SolveError::Store(_)) => ErrorKind::Io,
            Self::Solve(SolveError::Cancelled) | Self::Cancelled => ErrorKind::Cancelled,
            Self::Solve(_) => ErrorKind::Computation,
        }
    }

    /// Validation error with a single issue.
    pub fn invalid(issue: impl Into<String>) -> Self {
        Self::Validation {
            issues: vec![issue.into()],
        }
    }

    pub(crate) fn gdal(path: impl Into<PathBuf>) -> impl FnOnce(gdal::errors::GdalError) -> Self {
        let path = path.into();
        move |source| Self::Gdal { path, source }
    }

    pub(crate) fn file(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::File { path, source }
    }
}

impl From<SolveError> for Error {
    fn from(error: SolveError) -> Self {
        match error {
            SolveError::Cancelled => Self::Cancelled,
            other => Self::Solve(other),
        }
    }
}

/// Result of the I/O boundary.
pub type Result<T> = std::result::Result<T, Error>;
