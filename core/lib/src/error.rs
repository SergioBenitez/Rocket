//! Types representing various errors that can occur in a Rocket application.

use std::{io, fmt};
use std::sync::atomic::{Ordering, AtomicBool};

use yansi::Paint;
use figment::Profile;

/// An error that occurs during launch.
///
/// An `Error` is returned by [`launch()`](crate::Rocket::launch()) when
/// launching an application fails or, more rarely, when the runtime fails after
/// lauching.
///
/// # Panics
///
/// A value of this type panics if it is dropped without first being inspected.
/// An _inspection_ occurs when any method is called. For instance, if
/// `println!("Error: {}", e)` is called, where `e: Error`, the `Display::fmt`
/// method being called by `println!` results in `e` being marked as inspected;
/// a subsequent `drop` of the value will _not_ result in a panic. The following
/// snippet illustrates this:
///
/// ```rust
/// # let _ = async {
/// if let Err(error) = rocket::build().launch().await {
///     // This println "inspects" the error.
///     println!("Launch failed! Error: {}", error);
///
///     // This call to drop (explicit here for demonstration) will do nothing.
///     drop(error);
/// }
/// # };
/// ```
///
/// When a value of this type panics, the corresponding error message is pretty
/// printed to the console. The following illustrates this:
///
/// ```rust
/// # let _ = async {
/// let error = rocket::build().launch().await;
///
/// // This call to drop (explicit here for demonstration) will result in
/// // `error` being pretty-printed to the console along with a `panic!`.
/// drop(error);
/// # };
/// ```
///
/// # Usage
///
/// An `Error` value should usually be allowed to `drop` without inspection.
/// There are at least two exceptions:
///
///   1. If you are writing a library or high-level application on-top of
///      Rocket, you likely want to inspect the value before it drops to avoid a
///      Rocket-specific `panic!`. This typically means simply printing the
///      value.
///
///   2. You want to display your own error messages.
pub struct Error {
    handled: AtomicBool,
    kind: ErrorKind
}

/// The kind error that occurred.
///
/// In almost every instance, a launch error occurs because of an I/O error;
/// this is represented by the `Io` variant. A launch error may also occur
/// because of ill-defined routes that lead to collisions or because a fairing
/// encountered an error; these are represented by the `Collision` and
/// `FailedFairing` variants, respectively.
#[derive(Debug)]
pub enum ErrorKind {
    /// Binding to the provided address/port failed.
    Bind(io::Error),
    /// An I/O error occurred during launch.
    Io(io::Error),
    /// An I/O error occurred in the runtime.
    Runtime(Box<dyn std::error::Error + Send + Sync>),
    /// A valid [`Config`](crate::Config) could not be extracted from the
    /// configured figment.
    Config(figment::Error),
    /// Route collisions were detected.
    Collisions(crate::router::Collisions),
    /// Launch fairing(s) failed.
    FailedFairings(Vec<crate::fairing::Info>),
    /// The configuration profile is not debug but not secret key is configured.
    InsecureSecretKey(Profile),
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Error::new(kind)
    }
}

impl Error {
    #[inline(always)]
    pub(crate) fn new(kind: ErrorKind) -> Error {
        Error { handled: AtomicBool::new(false), kind }
    }

    #[inline(always)]
    fn was_handled(&self) -> bool {
        self.handled.load(Ordering::Acquire)
    }

    #[inline(always)]
    fn mark_handled(&self) {
        self.handled.store(true, Ordering::Release)
    }

    /// Retrieve the `kind` of the launch error.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::error::ErrorKind;
    ///
    /// # let _ = async {
    /// if let Err(error) = rocket::build().launch().await {
    ///     match error.kind() {
    ///         ErrorKind::Io(e) => println!("found an i/o launch error: {}", e),
    ///         e => println!("something else happened: {}", e)
    ///     }
    /// }
    /// # };
    /// ```
    #[inline]
    pub fn kind(&self) -> &ErrorKind {
        self.mark_handled();
        &self.kind
    }
}

impl std::error::Error for Error {  }

impl fmt::Display for ErrorKind {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::Bind(e) => write!(f, "binding failed: {}", e),
            ErrorKind::Io(e) => write!(f, "I/O error: {}", e),
            ErrorKind::Collisions(_) => "collisions detected".fmt(f),
            ErrorKind::FailedFairings(_) => "a launch fairing failed".fmt(f),
            ErrorKind::Runtime(e) => write!(f, "runtime error: {}", e),
            ErrorKind::InsecureSecretKey(_) => "insecure secret key config".fmt(f),
            ErrorKind::Config(_) => "failed to extract configuration".fmt(f),
        }
    }
}

impl fmt::Debug for Error {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.mark_handled();
        self.kind().fmt(f)
    }
}

impl fmt::Display for Error {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.mark_handled();
        write!(f, "{}", self.kind())
    }
}

impl Drop for Error {
    fn drop(&mut self) {
        if self.was_handled() {
            return
        }

        match self.kind() {
            ErrorKind::Bind(ref error) => {
                error_span!("bind_error", "Rocket failed to bind network socket to given address/port.").in_scope(|| {
                    info!(%error);
                });
                panic!("aborting due to socket bind error");
            }
            ErrorKind::Io(ref error) => {
                error_span!("io_error", "Rocket failed to launch due to an I/O error.").in_scope(|| {
                    info!(%error);
                });
                panic!("aborting due to i/o error");
            }
            ErrorKind::Collisions(ref collisions) => {
                fn log_collisions<T: fmt::Display>(kind: &str, collisions: &[(T, T)]) {
                    if collisions.is_empty() { return }

                    let span = error_span!("collisions", "Rocket failed to launch due to the following {} collisions:", kind);
                    let _e = span.enter();
                    for &(ref a, ref b) in collisions {
                        info!("{} {} {}", a, Paint::red("collides with").italic(), b)
                    }
                }

                log_collisions("route", &collisions.routes);
                log_collisions("catcher", &collisions.catchers);

                info!("Note: Route collisions can usually be resolved by ranking routes.");
                panic!("routing collisions detected");
            }
            ErrorKind::FailedFairings(ref failures) => {
                error_span!("fairing_error", "Rocket failed to launch due to failing fairings:").in_scope(|| {
                    for fairing in failures {
                        info!("{}", fairing.name);
                    }
                });
                panic!("aborting due to launch fairing failure");
            }
            ErrorKind::Runtime(ref error) => {
                error_span!("runtime_error", "An error occured in the runtime:").in_scope(|| {
                    info!(%error);
                });
                panic!("aborting due to runtime failure");
            }
            ErrorKind::InsecureSecretKey(ref profile) => {
                error_span!("insecure_secret_key", "secrets enabled in non-debug without `secret_key`").in_scope(|| {
                    info!("selected profile: {}", Paint::white(profile));
                    info!("disable `secrets` feature or configure a `secret_key`");
                });
                panic!("aborting due to insecure configuration")
            }
            ErrorKind::Config(ref error) => {
                crate::config::pretty_print_error(error.clone());
                panic!("aborting due to invalid configuration")
            }
        }
    }
}
