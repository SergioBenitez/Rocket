//! Tracing, telemetry, and logging.
//!
//! Rocket provides support for application-level diagnostics using
//! the [`tracing`] crate. `tracing` provides a _facade layer_ for scoped,
//! structured, application-level diagnostics. This means that diagnostic data
//! from Rocket applications, from Rocket itself, and from any dependencies that
//! use the [`tracing`] or [`log`] crates, can be emitted in a machine-readable
//! format and consumed in a wide variety of ways, including structured logging,
//! distributed tracing, and performance profiling.
//!
//! This module re-exports core components of the `tracing` API for use in
//! Rocket applications, and provides Rocket-specific APIs for use with
//! `tracing`.
//!
//! # Using Tracing
//!
//! Tracing's data model is based around two core concepts: [_spans_][spans] and
//! [_events_][events]. A span represents a _period of time_, with a beginning and
//! an end, during which a program was executing in a particular context or
//! performing a unit of work. An event represents a _momentary_ occurance
//! within a trace, comparable to a single log line.
//!
//! Spans and events are recorded using macros, the basics of which are likely
//! familiar to users of the [`log`] crate. The [`trace!`], [`debug!`],
//! [`info!`], [`warn!`], and [`error!`] macros record an event at a priority
//! level ranging from extremely verbose diagnostic information (`trace!`) to
//! high-priority warnings (`error!`). For example:
//!
//! ```
//! use rocket::trace;
//!
//! trace::trace!("Apollo 13 presently at 177,861 nautical miles away.");
//! trace::debug!("Velocity now reading 3,263 feet per second.");
//! trace::info!("13, we'd like you to stir up your cryo tanks.");
//! trace::warn!("Houston, we've got a Main Bus B undervolt.");
//! trace::error!("Houston, we are venting something out into space!");
//! ```
//!
//! An event consists of one or more key-value pairs, called _fields_, and/or a
//! textual, human-readable _message_. For example, this will record an event
//! at the `info` level, with two fields, named `answer` and `question`:
//!
//! ```
//! # use rocket::trace;
//! trace::info!(answer = 42, question = "life, the universe, and everything");
//! ```
//! The [`tracing` documentation][macros] provides details on how these macros are used.
//!
//! Spans may be recorded in a few different ways. Like events, they have a
//! priority level, and may have one or more fields. In addition, all spans also
//! have a _name_.
//!
//! Rocket's code generation will automatically generate spans for route and
//! catcher functions, so any events emitted within those functions or functions
//! they call will be annotated with the name of the handler or catcher
//! function. For example:
//! ```
//! # use rocket::get;
//! #[get("/hello-world")]
//! fn hello_world() -> String {
//!     // This event will occur within a span named `hello_world`.
//!     rocket::trace::info!("saying hello!");
//!
//!     "Hello world".to_string()
//! }
//! ```
//! Spans may be added to other functions, as well. The easiest way to
//! do this is to add the [`#[rocket::trace::instrument]`][instrument] attribute
//! to that function. For example:
//!
//! ```
//! # #[derive(Debug)] struct Planet;
//! // Calling this function will enter a new span named `jump_to_hyperspace`.
//! #[rocket::trace::instrument]
//! async fn jump_to_hyperspace(destination: Planet) {
//!     // This event will be recorded *within* the `jump_to_hyperspace` span.
//!     tracing::debug!("preparing to jump to hyperspace...");
//!
//!    // ...
//! }
//! ```
//! This will automatically create a span with the same name as the instrumented
//! function, and all the arguments to that function recorded as fields.
//! Additional arguments to `#[instrument]` allow customizing the span further.
//! See the [`tracing` crate's documentation](instrument) for details.
//!
//! In addition, spans may be created manually using the [`trace_span!`],
//! [`debug_span!`], [`info_span!`], [`warn_span!`], and [`error_span!`] macros.
//! Again, the [`tracing` documentation][macros] provides further details on how
//! to use these macros.
//!
//! # Customization
//!
//! Spans and events are recorded by a `tracing` component called a
//! [`Subscriber`], which implements a particular way of collecting and
//! recording trace data. By default, Rocket provides its own `Subscriber`
//! implementation, which logs events to the console. This `Subscriber` will be
//! installed when [`rocket::ignite`] is called.
//!
//! To override the default `Subscriber` with another implementation, simply
//! [set it as the default][default] prior to calling `rocket::ignite`. For
//! example:
//! ```
//! # type MySubscriber = tracing_subscriber::registry::Registry;
//! #[rocket::launch]
//! fn rocket() -> _ {
//!     let subscriber = MySubscriber::default();
//!     tracing::subscriber::set_global_default(subscriber)
//!         .expect("the global default subscriber should not have been set");
//!
//!     rocket::build()
//!         // ...
//! }
//! ```
//!
//! Since `tracing` data is structured and machine-readable, it may be collected
//! in a variety of ways. The `tracing` community provides [several crates] for
//! logging in several commonly-used formats, emitting distributed tracing data
//! to collectors like [OpenTelemetry] and [honeycomb.io], and for
//! [multiple][timing] [forms][flame] of performance profiling.
//!
//! The [`tracing-subscriber`] crate provides an abstraction for building
//! a `Subscriber` by composing multiple [`Layer`]s which implement different
//! ways of collecting traces. This allows applications to record the same trace
//! data in multiple ways.
//!
//! In addition to providing a default subscriber out of the box, Rocket also
//! exposes its default logging and filtering behavior as `Layer`s. This means
//! that users who would like to combine the default logging layer with layers
//! from other crates may do so. For example:
//!
//! ```rust
//! # use tracing_subscriber::Layer;
//! # #[derive(Default)] struct SomeLayer;
//! # impl<S: tracing::Subscriber + 'static> Layer<S> for SomeLayer {}
//! # #[derive(Default)] struct SomeOtherLayer;
//! # impl<S: tracing::Subscriber + 'static> Layer<S> for SomeOtherLayer {}
//! #[rocket::launch]
//! fn rocket() -> _ {
//!     use rocket::trace::prelude::*;
//!
//!     let figment = rocket::Config::figment();
//!     let config = rocket::Config::from(&figment);
//!
//!     // Configure our trace subscriber...
//!     tracing_subscriber::registry()
//!         // Add Rocket's default log formatter.
//!         .with(rocket::trace::logging_layer())
//!         // Add a custom layer...
//!         .with(SomeLayer::default())
//!         // ...and another custom layer.
//!         .with(SomeOtherLayer::default())
//!         // Filter what traces are enabled based on the Rocket config.
//!         .with(rocket::trace::filter_layer(config.log_level))
//!         // Set our subscriber as the default.
//!         .init();
//!
//!     rocket::custom(figment)
//!         // ...
//! }
//! ```
//!
//! [`tracing`]: https://docs.rs/tracing
//! [`log`]: https://docs.rs/log/
//! [spans]: https://docs.rs/tracing/latest/tracing/#spans
//! [events]: https://docs.rs/tracing/latest/tracing/#events
//! [`span!`]: https://docs.rs/tracing/latest/tracing/macro.span.html
//! [`event!`]: https://docs.rs/tracing/latest/tracing/macro.event.html
//! [`trace!`]: https://docs.rs/tracing/latest/tracing/macro.trace.html
//! [`debug!`]: https://docs.rs/tracing/latest/tracing/macro.debug.html
//! [`info!`]: https://docs.rs/tracing/latest/tracing/macro.info.html
//! [`warn!`]: https://docs.rs/tracing/latest/tracing/macro.warn.html
//! [`error!`]: https://docs.rs/tracing/latest/tracing/macro.error.html
//! [macros]: https://docs.rs/tracing/latest/tracing/index.html#using-the-macros
//! [instrument]: https://docs.rs/tracing/latest/tracing/attr.instrument.html
//! [`trace_span!`]: https://docs.rs/tracing/latest/tracing/macro.trace_span.html
//! [`debug_span!`]: https://docs.rs/tracing/latest/tracing/macro.debug_span.html
//! [`info_span!`]: https://docs.rs/tracing/latest/tracing/macro.info_span.html
//! [`warn_span!`]: https://docs.rs/tracing/latest/tracing/macro.warn_span.html
//! [`error_span!`]: https://docs.rs/tracing/latest/tracing/macro.error_span.html
//! [`rocket::ignite`]: crate::ignite
//! [default]: https://docs.rs/tracing/latest/tracing/#in-executables
//! [`Subscriber`]: https://docs.rs/tracing/latest/tracing/trait.Subscriber.html
//! [several crates]: https://github.com/tokio-rs/tracing#related-crates
//! [`tracing-subscriber`]: https://docs.rs/tracing-subscriber/
//! [`Layer`]: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/layer/trait.Layer.html
//! [OpenTelemetry]: https://crates.io/crates/tracing-opentelemetry
//! [honeycomb.io]: https://crates.io/crates/tracing-honeycomb
//! [timing]: https://crates.io/crates/tracing-timing
//! [flame]: https://crates.io/crates/tracing-flame
use tracing_subscriber::{
    fmt::{
        format::{self, FormatEvent, FormatFields},
        FmtContext, FormattedFields,
    },
    layer::Layer,
    prelude::*,
    registry::LookupSpan,
};

use std::fmt::{self, Write};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering::{Acquire, Release}};
use std::str::FromStr;

use yansi::Paint;
use serde::{de, Serialize, Serializer, Deserialize, Deserializer};

pub use tracing::{
    trace, debug, info, warn, error, trace_span, debug_span, info_span,
    warn_span, error_span, instrument,
};

pub use tracing_futures::Instrument;
pub use tracing_subscriber::{registry, EnvFilter as Filter};

/// A prelude for working with `tracing` in Rocket applications.
pub mod prelude {
    pub use tracing_subscriber::prelude::*;
    pub use super::Instrument as _;
}

/// Defines the maximum level of log messages to show.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum LogLevel {
    /// Only shows errors and warnings: `"critical"`.
    Critical,
    /// Shows errors, warnings, and some informational messages that are likely
    /// to be relevant when troubleshooting such as configuration: `"support"`.
    Support,
    /// Shows everything except debug and trace information: `"normal"`.
    Normal,
    /// Shows everything: `"debug"`.
    Debug,
    /// Shows nothing: "`"off"`".
    Off,
}

impl LogLevel {
    fn as_str(&self) -> &str {
        match self {
            LogLevel::Critical => "critical",
            LogLevel::Support => "support",
            LogLevel::Normal => "normal",
            LogLevel::Debug => "debug",
            LogLevel::Off => "off",
        }
    }
}

impl FromStr for LogLevel {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let level = match &*s.to_ascii_lowercase() {
            "critical" => LogLevel::Critical,
            "support" => LogLevel::Support,
            "normal" => LogLevel::Normal,
            "debug" => LogLevel::Debug,
            "off" => LogLevel::Off,
            _ => return Err("a log level (off, debug, normal, support, critical)"),
        };

        Ok(level)
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Serialize for LogLevel {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for LogLevel {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let string = String::deserialize(de)?;
        LogLevel::from_str(&string).map_err(|_| de::Error::invalid_value(
            de::Unexpected::Str(&string),
            &figment::error::OneOf( &["critical", "normal", "debug", "off"])
        ))
    }
}


/// Returns a Rocket filtering [`Layer`] based on the provided logging level.
///
/// The returned [`Layer`] can be added to another `tracing` subscriber to
/// configure it to filter spans and events based on the logging level
/// specified in the Rocket config.
///
/// Additional [filtering directives][dirs] may be added to the returned filter
/// layer in order to enable or disable specific targets.
///
/// # Examples
///
/// Using Rocket's filtering with a custom `tracing` subscriber:
///
/// ```
/// # type MySubscriber = tracing_subscriber::registry::Registry;
/// #[rocket::launch]
/// fn rocket() -> _ {
///     use rocket::trace::prelude::*;
///
///     let figment = rocket::Config::figment();
///     let config = rocket::Config::from(&figment);
///
///     // Use some `tracing` subscriber from another crate...
///     MySubscriber::default()
///         // ...but filter spans and events based on the Rocket
///         // config file.
///         .with(rocket::trace::filter_layer(config.log_level))
///         .init();
///
///     rocket::custom(figment)
///         // ...
/// }
/// ```
///
/// Adding additional directives to Rocket's default filter:
///
/// ```
/// #[rocket::launch]
/// fn rocket() -> _ {
///     use rocket::trace::prelude::*;
///
///     let figment = rocket::Config::figment();
///     let config = rocket::Config::from(&figment);
///
///     // Use Rocket's default filter for the configured log level...
///     let trace_filter = rocket::trace::filter_layer(config.log_level)
///         // ...but always enable the `DEBUG` level for `my_crate`.
///         .add_directive("my_crate=debug".parse().unwrap());
///
///     // Build a custom `tracing` subscriber...
///     rocket::trace::registry()
///         // ...using the default Rocket log formatter...
///         .with(rocket::trace::logging_layer())
///         // ...but replacing the default filter with our customized one.
///         .with(trace_filter)
///         .init();
///
///     rocket::custom(figment)
///         // ...
/// }
/// ```
///
/// [`Layer`]: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/layer/trait.Layer.html
/// [dirs]: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives
pub fn filter_layer(level: LogLevel) -> Filter {
    let filter_str = match level {
        LogLevel::Critical => "warn,hyper=off,rustls=off",
        LogLevel::Support => "warn,rocket::support=info,hyper=off,rustls=off",
        LogLevel::Normal => "info,hyper=off,rustls=off",
        LogLevel::Debug => "trace",
        LogLevel::Off => "off",
    };

    tracing_subscriber::filter::EnvFilter::try_new(filter_str)
        .expect("filter string must parse")
}

/// Returns a Rocket-style log formatting layer.
///
/// The returned layer can be added to a [`tracing-subscriber`
/// `Registry`][registry] to add Rocket-style log formatting in addition to
/// other [`Layer`s] providing different functionality.
///
/// For example:
///
/// ```
/// # type MySubscriber = tracing_subscriber::registry::Registry;
/// #[rocket::launch]
/// fn rocket() -> _ {
///     use rocket::trace::prelude::*;
///
///     let figment = rocket::Config::figment();
///     let config = rocket::Config::from(&figment);
///
///     // Use some `tracing` subscriber from another crate...
///     MySubscriber::default()
///         // ...but filter spans and events based on the Rocket
///         // config file.
///         .with(rocket::trace::filter_layer(config.log_level))
///         .init();
///
///     rocket::custom(figment)
///         // ...
/// }
/// ```
///
/// [`Layer`]: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/layer/trait.Layer.html
/// [`registry`]: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/registry/index.html
pub fn logging_layer<S>() -> impl Layer<S>
where
    S: tracing::Subscriber,
    S: for<'span> LookupSpan<'span>,
{
    let field_format = format::debug_fn(|writer, field, value| {
        // We'll format the field name and value separated with a colon.
        let name = field.name();
        if name == "message" {
            write!(writer, "{:?}", Paint::new(value).bold())
        } else {
            write!(writer, "{}: {:?}", field, Paint::default(value).bold())
        }
    })
    .delimited(", ")
    .display_messages();

    #[cfg(feature = "log")]
    let field_format = skip_log::SkipLogFields(field_format);

    tracing_subscriber::fmt::layer()
        .fmt_fields(field_format)
        // Configure the formatter to use `print!` rather than
        // `stdout().write_str(...)`, so that logs are captured by libtest's test
        // capturing.
        .with_test_writer()
        .event_format(EventFormat { last_id: AtomicU64::new(0), last_depth: AtomicU16::new(0) })
}

pub(crate) fn try_init(config: &crate::Config) -> bool {
    if config.log_level == LogLevel::Off {
        Paint::disable();
        return false;
    }

    if !atty::is(atty::Stream::Stdout)
        || (cfg!(windows) && !Paint::enable_windows_ascii())
        || !config.cli_colors
    {
        Paint::disable();
    }

    // Try to enable a `log` compatibility layer to collect logs from
    // dependencies using the `log` crate as `tracing` diagnostics.
    #[cfg(feature = "log")]
    if try_init_log(config.log_level).is_err() {
        // We failed to set the default `log` logger. This means that the user
        // has already set a `log` logger. In that case, don't try to set up a
        // `tracing` subscriber as well --- instead, Rocket's `tracing` events
        // will be recorded as `log` records.
        Paint::disable();
        return false;
    }

    let success = tracing::subscriber::set_global_default(tracing_subscriber::registry()
        .with(logging_layer())
        .with(filter_layer(config.log_level))
    )
        .is_ok();

    if !success {
        // Something else already registered a tracing subscriber;
        // don't assume we can use Paint.
        Paint::disable();
    }

    success
}

pub trait PaintExt {
    fn emoji(item: &str) -> Paint<&str>;
}

impl PaintExt for Paint<&str> {
    /// Paint::masked(), but hidden on Windows due to broken output. See #1122.
    fn emoji(_item: &str) -> Paint<&str> {
        #[cfg(windows)] { Paint::masked("") }
        #[cfg(not(windows))] { Paint::masked(_item) }
    }
}


struct EventFormat {
    last_id: AtomicU64,
    last_depth: AtomicU16,
}

impl<S, N> FormatEvent<S, N> for EventFormat
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    for<'a> FmtContext<'a, S, N>: FormatFields<'a>,
{
    fn format_event(
        &self,
        cx: &FmtContext<'_, S, N>,
        writer: &mut dyn fmt::Write,
        event: &tracing::Event<'_>,
    ) -> fmt::Result {
        let mut indent = String::new();
        let mut root_id = None;
        let mut root_changed = false;
        let mut depth = 0;

        let prev_depth = self.last_depth.load(Acquire);

        // FIXME: explain or replace this
        fn hash_id(id: tracing::Id) -> u16 {
            let id = id.into_u64();
            let [z, y, x, w, v, u, t, s] = id.to_le_bytes();

            (z as u16) ^ (y as u16) ^
                (x as u16) ^ (w as u16) ^
                (v as u16) << 8 ^ (u as u16) << 8 ^
                (t as u16) << 8 ^ (s as u16) << 8
        }

        cx.visit_spans(|span| {
            let is_root = root_id.is_none();
            if is_root {
                root_id = Some(span.id());
                if span.id().into_u64() != self.last_id.load(Acquire) {
                    root_changed = true;
                }
            }
            depth += 1;

            if root_changed || depth > prev_depth {
                if !indent.is_empty() {
                    write!(writer, "{}{} ", indent, Paint::default("=>").bold())?;
                }
                let meta = span.metadata();
                let exts = span.extensions();
                if let Some(fields) = exts.get::<FormattedFields<N>>() {
                    // If the span has a human-readable message, print that
                    // instead of the span's name (so that we can get nice emojis).
                    if meta.fields().iter().any(|field| field.name() == "message") {
                        if span.name() == "request" {
                            with_meta(writer, meta, format_args!("[{:x}] {}", hash_id(span.id()), &fields.fields))?;
                        } else {
                            with_meta(writer, meta, &fields.fields)?;
                        }
                    } else {
                        with_meta(writer, meta, format_args!("{} {}", Paint::new(span.name()).bold(), &fields.fields))?;
                    }
                } else {
                    with_meta(writer, span.metadata(),  Paint::new(span.name()).bold())?;
                }
            }

            indent.push_str("    ");
            Ok(())
        })?;

        if !indent.is_empty() {
            write!(writer, "{}{} ", indent, Paint::default("=>").bold())?;
        }

        // If the `log` compatibility feature is enabled, normalize metadata
        // from `log` records that's emitted as fields.
        #[cfg(feature = "log")]
        let normalized_meta = tracing_log::NormalizeEvent::normalized_metadata(event);
        #[cfg(feature = "log")]
        let event_meta = normalized_meta.as_ref().unwrap_or_else(|| event.metadata());
        #[cfg(not(feature = "log"))]
        let event_meta = event.metadata();

        with_meta(
            writer,
            event_meta,
            DisplayFields {
                fmt: cx,
                event,
            },
        )?;
        if root_changed {
            self.last_id.store(root_id.unwrap().into_u64(), Release);
        }
        self.last_depth.store(depth, Release);
        Ok(())
    }
}


struct DisplayFields<'a, F, R> {
    fmt: &'a F,
    event: &'a R,
}

impl<'a, F, R> fmt::Display for DisplayFields<'a, F, R>
where
    F: for<'writer> FormatFields<'writer>,
    R: tracing_subscriber::field::RecordFields,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt.format_fields(f, self.event)
    }
}

fn with_meta(
    writer: &mut dyn Write,
    meta: &tracing::Metadata<'_>,
    f: impl fmt::Display,
) -> fmt::Result {

    struct WithFile<'a, F> {
        meta: &'a tracing::Metadata<'a>,
        f: F,
    }

    impl<F: fmt::Display> fmt::Display for WithFile<'_, F> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match (self.meta.file(), self.meta.line()) {
                (Some(file), Some(line)) => write!(
                    f,
                    "{}\n    {} {}:{}",
                    self.f,
                    Paint::new("-->").bold(),
                    file,
                    line
                ),
                (Some(file), None) => write!(
                    f,
                    "{}\n    {} {}",
                    self.f,
                    Paint::new("-->").bold(),
                    file,
                ),
                _ => write!(f,  "{}", self.f),
            }
        }
    }


    match *meta.level() {
        tracing::Level::INFO => writeln!(writer, "{}", Paint::blue(f).wrap()),
        tracing::Level::ERROR => writeln!(
            writer,
            "{} {}",
            Paint::red("Error:").bold(),
            Paint::red(f).wrap()
        ),
        tracing::Level::WARN => writeln!(
            writer,
            "{} {}",
            Paint::yellow("Warning:").bold(),
            Paint::yellow(f).wrap()
        ),
        tracing::Level::TRACE => writeln!(writer, "{}", Paint::magenta(WithFile { meta, f }).wrap()),
        tracing::Level::DEBUG => writeln!(writer, "{}", Paint::blue(WithFile { meta, f }).wrap()),
    }
}

/// Set up `tracing`/`log` compatibility.
#[cfg(feature = "log")]
fn try_init_log(filter: LogLevel) -> Result<(), impl std::error::Error> {
    use log_crate::LevelFilter;

    let builder = tracing_log::LogTracer::builder()
        // Hyper and Rocket both use `tracing`. If `tracing`'s `log` feature
        // is enabled and the `tracing` macros in Hyper and Rocket also emit
        // `log` records, ignore them, because the native `tracing` events
        // will already be collected.
        .ignore_all(vec!["rocket", "hyper", "tracing::span"]);
    let builder = match filter {
        LogLevel::Critical | LogLevel::Support => builder
            .ignore_crate("rustls")
            // Set the max level for all `log` records to Warn. Rocket's
            // `launch` events will be collected by the native `tracing`
            // subscriber, so we don't need to allow `log` records at Info
            // in order to see them.
            .with_max_level(LevelFilter::Warn),
        LogLevel::Normal => builder.ignore_crate("rustls").with_max_level(LevelFilter::Info),
        LogLevel::Debug => builder.with_max_level(LevelFilter::Trace),
        LogLevel::Off => return Ok(()),
    };
    builder.init()
}

#[cfg(feature = "log")]
mod skip_log {
    use tracing::field::Field;
    use tracing_subscriber::field::{MakeVisitor, RecordFields, Visit, VisitFmt, VisitOutput};

    use super::*;

    // This struct along with SkipLogVisitor suppress the output of any fields
    // whose names start with "log."; it's used under cfg(feature="log") to clean
    // up the output. This complements NormalizeEvent, which converts log fields
    // into event metadata but does not remove the fields it used.
    pub(crate) struct SkipLogFields<V>(pub V);

    impl<'a, V: MakeVisitor<&'a mut (dyn Write + 'a)>> MakeVisitor<&'a mut (dyn Write + 'a)> for SkipLogFields<V> {
        type Visitor = SkipLogFields<V::Visitor>;

        fn make_visitor(&self, target: &'a mut dyn Write) -> Self::Visitor {
            SkipLogFields(self.0.make_visitor(target))
        }
    }

    macro_rules! forward_record_fns {
        ($($fn_name:ident ( $type:ty ) ),*) => {
            $(
                fn $fn_name (&mut self, field: &Field, value: $type) {
                    if field.name().starts_with("log.") { return; }
                    self.0.$fn_name(field, value)
                }
            )*
        };
    }

    impl<V: Visit> Visit for SkipLogFields<V> {
        forward_record_fns!(
            record_debug(&dyn std::fmt::Debug),
            record_i64(i64),
            record_u64(u64),
            record_str(&str),
            record_bool(bool)
        );
    }

    impl<V: VisitFmt> VisitFmt for SkipLogFields<V> {
        fn writer(&mut self) -> &mut dyn Write {
            self.0.writer()
        }
    }

    impl<V: VisitOutput<O>, O> VisitOutput<O> for SkipLogFields<V> {
        fn visit<R>(self, fields: &R) -> O where R: RecordFields, Self: Sized {
            self.0.visit(fields)
        }

        fn finish(self) -> O {
            self.0.finish()
        }
    }
}
