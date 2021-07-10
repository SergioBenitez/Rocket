use std::fmt;

use rocket::figment;

/// A general error type designed for the `Poolable` trait.
///
/// [`Pool::initialize`] can return an error for any of several reasons:
///
///   * Missing or incorrect configuration, including some syntax errors
///   * An error connecting to the database.
///
/// [`Pool::initialize`]: crate::Pool::initialize
#[derive(Debug)]
pub enum Error<E> {
    /// A database-specific error occurred
    Db(E),

    /// An error occurred in the configuration
    Figment(figment::Error),

    /// Required fairing was not attached
    UnattachedFairing,
}

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Db(e) => e.fmt(f),
            Error::Figment(e) => write!(f, "bad configuration: {}", e),
            Error::UnattachedFairing => write!(f, "required database fairing was not attached"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for Error<E> {}

impl<E> From<figment::Error> for Error<E> {
    fn from(e: figment::Error) -> Self {
        Self::Figment(e)
    }
}
