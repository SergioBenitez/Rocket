use std::path::PathBuf;
use std::net::{IpAddr, Ipv4Addr};

use figment::{Figment, Profile, Provider, Metadata, error::Result};
use figment::providers::{Serialized, Env, Toml, Format};
use figment::value::{Map, Dict};
use serde::{Deserialize, Serialize};
use yansi::Paint;

use crate::config::{TlsConfig, LogLevel};
use crate::request::{self, Request, FromRequest};
use crate::data::Limits;

#[cfg(feature = "secrets")]
use crate::config::SecretKey;

/// Rocket server configuration.
///
/// See the [module level docs](crate::config) as well as the [configuration
/// guide] for further details.
///
/// [configuration guide]: https://rocket.rs/master/guide/configuration/
///
/// # Defaults
///
/// All configuration values have a default, documented in the [fields](#fields)
/// section below. [`Config::debug_default()`] returns the default values for
/// the debug profile while [`Config::release_default()`] the default values for
/// the release profile. The [`Config::default()`] method automatically selects
/// the appropriate of the two based on the selected profile. With the exception
/// of `log_level`, which is `normal` in `debug` and `critical` in `release`,
/// and `secret_key`, which is regenerated from a random value if not set in
/// "debug" mode only, all default values are identical in all profiles.
///
/// # Provider Details
///
/// `Config` is a Figment [`Provider`] with the following characteristics:
///
///   * **Profile**
///
///     The profile is set to the value of the `profile` field.
///
///   * **Metadata**
///
///     This provider is named `Rocket Config`. It does not specify a
///     [`Source`](figment::Source) and uses default interpolatation.
///
///   * **Data**
///
///     The data emitted by this provider are the keys and values corresponding
///     to the fields and values of the structure. The dictionary is emitted to
///     the "default" meta-profile.
///
/// Note that these behaviors differ from those of [`Config::figment()`].
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    /// The selected profile. **(default: _debug_ `debug` / _release_ `release`)**
    ///
    /// **Note:** This field is never serialized nor deserialized. When part of
    /// a `Config` `Provider`, it is emitted as the profile to select on the
    /// merged-into Figment. When a `Config` is extracted, this field is set to
    /// the extracting Figment's selected `Profile`.
    #[serde(skip)]
    pub profile: Profile,
    /// IP address to serve on. **(default: `127.0.0.1`)**
    pub address: IpAddr,
    /// Port to serve on. **(default: `8000`)**
    pub port: u16,
    /// Number of threads to use for executing futures. **(default: `num_cores`)**
    pub workers: usize,
    /// Keep-alive timeout in seconds; disabled when `0`. **(default: `5`)**
    pub keep_alive: u32,
    /// Streaming read size limits. **(default: [`Limits::default()`])**
    pub limits: Limits,
    /// The TLS configuration, if any. **(default: `None`)**
    pub tls: Option<TlsConfig>,
    /// The secret key for signing and encrypting. **(default: `0`)**
    ///
    /// **Note:** This field _always_ serializes as a 256-bit array of `0`s to
    /// aid in preventing leakage of the secret key.
    #[cfg(feature = "secrets")]
    #[cfg_attr(nightly, doc(cfg(feature = "secrets")))]
    #[serde(serialize_with = "SecretKey::serialize_zero")]
    pub secret_key: SecretKey,
    /// The directory to store temporary files in. **(default:
    /// [`std::env::temp_dir`]).
    pub temp_dir: PathBuf,
    /// Max level to log. **(default: _debug_ `normal` / _release_ `critical`)**
    pub log_level: LogLevel,
    /// Whether to use colors and emoji when logging. **(default: `true`)**
    #[serde(deserialize_with = "figment::util::bool_from_str_or_int")]
    pub cli_colors: bool,
    /// Whether `ctrl-c` initiates a server shutdown. **(default: `true`)**
    #[serde(deserialize_with = "figment::util::bool_from_str_or_int")]
    pub ctrlc: bool,
}

impl Default for Config {
    /// Returns the default configuration based on the Rust compilation profile.
    /// This is [`Config::debug_default()`] in `debug` and
    /// [`Config::release_default()`] in `release`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::Config;
    ///
    /// let config = Config::default();
    /// ```
    fn default() -> Config {
        #[cfg(debug_assertions)] { Config::debug_default() }
        #[cfg(not(debug_assertions))] { Config::release_default() }
    }
}

impl Config {

    const DEPRECATED_KEYS: &'static [(&'static str, Option<&'static str>)] = &[
        ("env", Some(Self::PROFILE)), ("log", Some(Self::LOG_LEVEL)),
    ];

    const DEPRECATED_PROFILES: &'static [(&'static str, Option<&'static str>)] = &[
        ("dev", Some("debug")), ("prod", Some("release")),
    ];

    /// Returns the default configuration for the `debug` profile, _irrespective
    /// of the Rust compilation profile_ and `ROCKET_PROFILE`.
    ///
    /// This may differ from the configuration used by default,
    /// [`Config::default()`], which is selected based on the Rust compilation
    /// profile. See [defaults](#defaults) and [provider
    /// details](#provider-details) for specifics.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::Config;
    ///
    /// let config = Config::debug_default();
    /// ```
    pub fn debug_default() -> Config {
        Config {
            profile: Self::DEBUG_PROFILE,
            address: Ipv4Addr::new(127, 0, 0, 1).into(),
            port: 8000,
            workers: num_cpus::get(),
            keep_alive: 5,
            limits: Limits::default(),
            tls: None,
            #[cfg(feature = "secrets")]
            secret_key: SecretKey::zero(),
            temp_dir: std::env::temp_dir(),
            log_level: LogLevel::Normal,
            cli_colors: true,
            ctrlc: true,
        }
    }

    /// Returns the default configuration for the `release` profile,
    /// _irrespective of the Rust compilation profile_ and `ROCKET_PROFILE`.
    ///
    /// This may differ from the configuration used by default,
    /// [`Config::default()`], which is selected based on the Rust compilation
    /// profile. See [defaults](#defaults) and [provider
    /// details](#provider-details) for specifics.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::Config;
    ///
    /// let config = Config::release_default();
    /// ```
    pub fn release_default() -> Config {
        Config {
            profile: Self::RELEASE_PROFILE,
            log_level: LogLevel::Critical,
            ..Config::debug_default()
        }
    }

    /// Returns the default provider figment used by [`rocket::build()`].
    ///
    /// The default figment reads from the following sources, in ascending
    /// priority order:
    ///
    ///   1. [`Config::default()`] (see [defaults](#defaults))
    ///   2. `Rocket.toml` _or_ filename in `ROCKET_CONFIG` environment variable
    ///   3. `ROCKET_` prefixed environment variables
    ///
    /// The profile selected is the value set in the `ROCKET_PROFILE`
    /// environment variable. If it is not set, it defaults to `debug` when
    /// compiled in debug mode and `release` when compiled in release mode.
    ///
    /// [`rocket::build()`]: crate::build()
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::Config;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct MyConfig {
    ///     app_key: String,
    /// }
    ///
    /// let my_config = Config::figment().extract::<MyConfig>();
    /// ```
    pub fn figment() -> Figment {
        Figment::from(Config::default())
            .select(Profile::from_env_or("ROCKET_PROFILE", Self::DEFAULT_PROFILE))
            .merge(Toml::file(Env::var_or("ROCKET_CONFIG", "Rocket.toml")).nested())
            .merge(Env::prefixed("ROCKET_").ignore(&["PROFILE"]).global())
    }

    /// Attempts to extract a `Config` from `provider`, returning the result.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::Config;
    /// use rocket::figment::providers::{Toml, Format, Env};
    ///
    /// // Use Rocket's default `Figment`, but allow values from `MyApp.toml`
    /// // and `MY_APP_` prefixed environment variables to supersede its values.
    /// let figment = Config::figment()
    ///     .merge(("some-thing", 123))
    ///     .merge(Env::prefixed("CONFIG_"));
    ///
    /// let config = Config::try_from(figment);
    /// ```
    pub fn try_from<T: Provider>(provider: T) -> Result<Self> {
        let figment = Figment::from(provider);
        let mut config = figment.extract::<Self>()?;
        config.profile = figment.profile().clone();
        Ok(config)
    }

    /// Extract a `Config` from `provider`, panicking if extraction fails.
    ///
    /// # Panics
    ///
    /// If extraction fails, prints an error message indicating the failure and
    /// panics. For a version that doesn't panic, use [`Config::try_from()`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::Config;
    /// use rocket::figment::providers::{Toml, Format, Env};
    ///
    /// // Use Rocket's default `Figment`, but allow values from `MyApp.toml`
    /// // and `MY_APP_` prefixed environment variables to supersede its values.
    /// let figment = Config::figment()
    ///     .merge(Toml::file("MyApp.toml").nested())
    ///     .merge(Env::prefixed("MY_APP_"));
    ///
    /// let config = Config::from(figment);
    /// ```
    pub fn from<T: Provider>(provider: T) -> Self {
        Self::try_from(provider).unwrap_or_else(|e| {
            pretty_print_error(e);
            panic!("aborting due to configuration error(s)")
        })
    }

    /// Returns `true` if TLS is enabled.
    ///
    /// TLS is enabled when the `tls` feature is enabled and TLS has been
    /// configured.
    ///
    /// # Example
    ///
    /// ```rust
    /// let config = rocket::Config::default();
    /// if config.tls_enabled() {
    ///     println!("TLS is enabled!");
    /// } else {
    ///     println!("TLS is disabled.");
    /// }
    /// ```
    pub fn tls_enabled(&self) -> bool {
        cfg!(feature = "tls") && self.tls.is_some()
    }

    pub(crate) fn pretty_print(&self, figment: &Figment) {
        use crate::trace::PaintExt;

        let span = info_span!(target: "rocket::support", "configured",
            "{}Configured for {}.", Paint::emoji("🔧 "), figment.profile());
        let _e = span.enter();
        info!(target: "rocket::support", address = %&self.address);
        info!(target: "rocket::support", port = %&self.port);
        info!(target: "rocket::support", workers = %self.workers);

        let ka = self.keep_alive;
        if ka > 0 {
            info!(target: "rocket::support", keep_alive = %format!("{}s", ka));
        } else {
            info!(target: "rocket::support", keep_alive = %"disabled");
        }

        info!(target: "rocket::support", limits = %&self.limits);
        match self.tls_enabled() {
            true => info!(target: "rocket::support", tls = %"enabled"),
            false => info!(target: "rocket::support", tls = %"disabled"),
        }

        #[cfg(feature = "secrets")] {
            info!(target: "rocket::support", secret_key = ?&self.secret_key);
            if !self.secret_key.is_provided() {
                warn_span!(target: "rocket::support", "missing_secret_key", "secrets enabled without a stable `secret_key`").in_scope(|| {
                    info!(target: "rocket::support", "disable `secrets` feature or configure a `secret_key`");
                    info!(target: "rocket::support", "this becomes an {} in non-debug profiles", Paint::red("error"));
                });
            }
        }

        info!(target: "rocket::support", temp_dir = %self.temp_dir.display());
        info!(target: "rocket::support", log_level = %self.log_level);
        info!(target: "rocket::support", cli_colors = %&self.cli_colors);

        // Check for now depreacted config values.
        for (key, replacement) in Self::DEPRECATED_KEYS {
            if let Some(md) = figment.find_metadata(key) {
                let span = warn_span!(target: "rocket::support", "deprecated_key", "found value for deprecated config key `{}`", Paint::white(key));
                let _e = span.enter();
                if let Some(ref source) = md.source {
                    info!(target: "rocket::support", "in {} {}", Paint::white(source), md.name);
                }

                if let Some(new_key) = replacement {
                    info!(target: "rocket::support", "key has been by replaced by `{}`", Paint::white(new_key));
                }
            }
        }

        // Check for now removed config values.
        for (prefix, replacement) in Self::DEPRECATED_PROFILES {
            if let Some(profile) = figment.profiles().find(|p| p.starts_with(prefix)) {
                let span = warn_span!(target: "rocket::support", "deprecated_profile", "found set deprecated profile `{}`", Paint::white(profile));
                let _e = span.enter();
                if let Some(new_profile) = replacement {
                    info!(target: "rocket::support", "profile has been by replaced by `{}`", Paint::white(new_profile));
                } else {
                    info!(target: "rocket::support", "profile `{}` has no special meaning", profile);
                }
            }
        }
    }
}

impl Config {
    /// The default debug profile: `debug`.
    pub const DEBUG_PROFILE: Profile = Profile::const_new("debug");

    /// The default release profile: `release`.
    pub const RELEASE_PROFILE: Profile = Profile::const_new("release");

    /// The default profile: "debug" on `debug`, "release" on `release`.
    #[cfg(debug_assertions)]
    pub const DEFAULT_PROFILE: Profile = Self::DEBUG_PROFILE;

    /// The default profile: "debug" on `debug`, "release" on `release`.
    #[cfg(not(debug_assertions))]
    pub const DEFAULT_PROFILE: Profile = Self::RELEASE_PROFILE;

}

impl Config {
    /// The stringy parameter name for setting/extracting [`Config::profile`].
    ///
    /// This isn't `pub` because setting it directly does nothing.
    const PROFILE: &'static str = "profile";

    /// The stringy parameter name for setting/extracting [`Config::address`].
    pub const ADDRESS: &'static str = "address";

    /// The stringy parameter name for setting/extracting [`Config::port`].
    pub const PORT: &'static str = "port";

    /// The stringy parameter name for setting/extracting [`Config::workers`].
    pub const WORKERS: &'static str = "workers";

    /// The stringy parameter name for setting/extracting [`Config::keep_alive`].
    pub const KEEP_ALIVE: &'static str = "keep_alive";

    /// The stringy parameter name for setting/extracting [`Config::limits`].
    pub const LIMITS: &'static str = "limits";

    /// The stringy parameter name for setting/extracting [`Config::tls`].
    pub const TLS: &'static str = "tls";

    /// The stringy parameter name for setting/extracting [`Config::secret_key`].
    pub const SECRET_KEY: &'static str = "secret_key";

    /// The stringy parameter name for setting/extracting [`Config::temp_dir`].
    pub const TEMP_DIR: &'static str = "temp_dir";

    /// The stringy parameter name for setting/extracting [`Config::log_level`].
    pub const LOG_LEVEL: &'static str = "log_level";

    /// The stringy parameter name for setting/extracting [`Config::ctrlc`].
    pub const CTRLC: &'static str = "ctrlc";
}

impl Provider for Config {
    fn metadata(&self) -> Metadata {
        Metadata::named("Rocket Config")
    }

    #[track_caller]
    fn data(&self) -> Result<Map<Profile, Dict>> {
        #[allow(unused_mut)]
        let mut map: Map<Profile, Dict> = Serialized::defaults(self).data()?;

        // We need to special-case `secret_key` since its serializer zeroes.
        #[cfg(feature = "secrets")]
        if !self.secret_key.is_zero() {
            if let Some(map) = map.get_mut(&Profile::Default) {
                map.insert("secret_key".into(), self.secret_key.key.master().into());
            }
        }

        Ok(map)
    }

    fn profile(&self) -> Option<Profile> {
        Some(self.profile.clone())
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for &'r Config {
    type Error = std::convert::Infallible;

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        request::Outcome::Success(req.rocket().config())
    }
}

#[doc(hidden)]
pub fn pretty_print_error(error: figment::Error) {
    use figment::error::{Kind, OneOf};

    let mut config = Config::debug_default();
    config.log_level = LogLevel::Debug;
    crate::trace::try_init(&config);

    error!("Rocket configuration extraction from provider failed.");
    for e in error {
        fn w<T: std::fmt::Display>(v: T) -> Paint<T> { Paint::white(v) }

        let span = match &e.kind {
            Kind::Message(msg) => error_span!("config_error", "{}", msg),
            Kind::InvalidType(v, exp) => {
                error_span!("config_error", "invalid type: found {}, expected {}", w(v), w(exp))
            }
            Kind::InvalidValue(v, exp) => {
                error_span!("config_error", "invalid value {}, expected {}", w(v), w(exp))
            },
            Kind::InvalidLength(v, exp) => {
                error_span!("config_error", "invalid length {}, expected {}", w(v), w(exp))
            },
            Kind::UnknownVariant(v, exp) => {
                error_span!("config_error", "unknown variant: found `{}`, expected `{}`", w(v), w(OneOf(exp)))
            }
            Kind::UnknownField(v, exp) => {
                error_span!("config_error", "unknown field: found `{}`, expected `{}`", w(v), w(OneOf(exp)))
            }
            Kind::MissingField(v) => {
                error_span!("config_error", "missing field `{}`", w(v))
            }
            Kind::DuplicateField(v) => {
                error_span!("config_error", "duplicate field `{}`", w(v))
            }
            Kind::ISizeOutOfRange(v) => {
                error_span!("config_error", "signed integer `{}` is out of range", w(v))
            }
            Kind::USizeOutOfRange(v) => {
                error_span!("config_error", "unsigned integer `{}` is out of range", w(v))
            }
            Kind::Unsupported(v) => {
                error_span!("config_error", "unsupported type `{}`", w(v))
            }
            Kind::UnsupportedKey(a, e) => {
                error_span!("config_error", "unsupported type `{}` for key: must be `{}`", w(a), w(e))
            }
        };

        let _e = span.enter();

        if let (Some(ref profile), Some(ref md)) = (&e.profile, &e.metadata) {
            if !e.path.is_empty() {
                let key = md.interpolate(profile, &e.path);
                info!(key = %w(&key));
            }
        }

        if let Some(md) = e.metadata {
            if let Some(source) = md.source {
                info!("in {} {}", w(&source), &md.name);
            } else {
                info!("in {}", w(&md.name));
            }
        }
    }
}
