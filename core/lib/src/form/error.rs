//! Form error types.

use std::{fmt, io};
use std::num::{ParseIntError, ParseFloatError};
use std::str::{Utf8Error, ParseBoolError};
use std::net::AddrParseError;
use std::borrow::Cow;

use serde::{Serialize, ser::{Serializer, SerializeStruct}};

use crate::http::Status;
use crate::form::name::{NameBuf, Name};
use crate::data::ByteUnit;

/// A collection of [`Error`]s.
///
/// `Errors` is a thin wrapper around a `Vec<Error>` with convenient methods for
/// modifying the internal `Error`s. It `Deref`s and `DerefMut`s to
/// `Vec<Error>` for transparent access to the underlying vector.
///
/// # Matching Errors to Fields
///
/// To find the errors that correspond to a given field, use
/// [`Error::is_for()`]. For example, to get all of the errors that correspond
/// to the field `foo.bar`, you might write the following:
///
/// ```rust
/// use rocket::form::Errors;
///
/// let errors = Errors::new();
/// let errors_for_foo = errors.iter().filter(|e| e.is_for("foo.bar"));
/// ```
///
/// ## Constructing
///
/// An `Errors` can be constructed from anything that an `Error` can be
/// constructed from. This includes [`Error`], [`ErrorKind`], and all of the
/// types an `ErrorKind` can be constructed from. See
/// [`ErrorKind`](ErrorKind#constructing) for the full list.
///
/// ```rust
/// use rocket::form;
///
/// fn at_most_10() -> form::Result<'static, usize> {
///     // Using `From<PartIntError> => ErrorKind::Int => Errors`.
///     let i: usize = "foo".parse()?;
///
///     if i > 10 {
///         // `(Option<isize>, Option<isize>) => ErrorKind::OutOfRange => Errors`
///         return Err((None, Some(10isize)).into());
///     }
///
///     Ok(i)
/// }
/// ```
#[derive(Default, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Errors<'v>(Vec<Error<'v>>);

/// A form error, potentially tied to a specific form field.
///
/// An `Error` is returned by [`FromForm`], [`FromFormField`], and [`validate`]
/// procedures, typically as a collection of [`Errors`]. It potentially
/// identifies a specific field that triggered the error via [`Error::name`] and
/// the field's value via [`Error::value`].
///
/// An `Error` can occur because of a field's value that failed to parse or
/// because other parts of a field or form were malformed; the [`Error::entity`]
/// identifies the part of the form that resulted in the error.
///
/// [`FromForm`]: crate::form::FromForm
/// [`FromFormField`]: crate::form::FromFormField
/// [`validate`]: crate::form::validate
///
/// # Constructing
///
/// An `Error` can be constructed via [`Error::validation()`],
/// [`Error::custom()`], or anything that an [`ErrorKind`] can be constructed
/// from. See [`ErrorKind`](ErrorKind#constructing).
///
/// ```rust
/// use rocket::form::Error;
///
/// fn at_most_10_not_even() -> Result<usize, Error<'static>> {
///     // Using `From<PartIntError> => ErrorKind::Int`.
///     let i: usize = "foo".parse()?;
///
///     if i > 10 {
///         // `From<(Option<isize>, Option<isize>)> => ErrorKind::OutOfRange`
///         return Err((None, Some(10isize)).into());
///     } else if i % 2 == 0 {
///         return Err(Error::validation("integer cannot be even"));
///     }
///
///     Ok(i)
/// }
/// ```
///
/// # Setting Field Metadata
///
/// When implementing [`FromFormField`], nothing has to be done for a field's
/// metadata to be set: the blanket [`FromForm`] implementation sets it
/// automatically.
///
/// When constructed from an `ErrorKind`, the entity is set to
/// [`Entity::default_for()`] by default. Occasionally, the error's `entity` may
/// need to be set manually. Return what would be useful to the end-consumer.
///
/// # Matching Errors to Fields
///
/// To determine whether an error corresponds to a given field, use
/// [`Error::is_for()`]. For example, to get all of the errors that correspond
/// to the field `foo.bar`, you might write the following:
///
/// ```rust
/// use rocket::form::Errors;
///
/// let errors = Errors::new();
/// let errors_for_foo = errors.iter().filter(|e| e.is_for("foo.bar"));
/// ```
///
/// # Serialization
///
/// When a value of this type is serialized, a `struct` or map with the
/// following fields is emitted:
///
/// | field    | type           | description                                      |
/// |----------|----------------|--------------------------------------------------|
/// | `name`   | `Option<&str>` | the erroring field's name, if known              |
/// | `value`  | `Option<&str>` | the erroring field's value, if known             |
/// | `entity` | `&str`         | string representation of the erroring [`Entity`] |
/// | `msg`    | `&str`         | concise message of the error                     |
#[derive(Debug, PartialEq)]
pub struct Error<'v> {
    /// The name of the field, if it is known.
    pub name: Option<NameBuf<'v>>,
    /// The field's value, if it is known.
    pub value: Option<Cow<'v, str>>,
    /// The kind of error that occurred.
    pub kind: ErrorKind<'v>,
    /// The entitiy that caused the error.
    pub entity: Entity,
}

/// The kind of form error that occurred.
///
/// ## Constructing
///
/// An `ErrorKind` can be constructed directly or via a `From` of the following
/// types:
///
///   * `(Option<u64>, Option<u64>)` => [`ErrorKind::InvalidLength`]
///   * `(Option<ByteUnit>, Option<ByteUnit>)` => [`ErrorKind::InvalidLength`]
///   * `(Option<isize>, Option<isize>)` => [`ErrorKind::OutOfRange`]
///   * `&[Cow<'_, str>]` or `Vec<Cow<'_, str>>` => [`ErrorKind::InvalidChoice`]
///   * [`Utf8Error`] => [`ErrorKind::Utf8`]
///   * [`ParseIntError`] => [`ErrorKind::Int`]
///   * [`ParseFloatError`] => [`ErrorKind::Float`]
///   * [`ParseBoolError`] => [`ErrorKind::Bool`]
///   * [`AddrParseError`] => [`ErrorKind::Addr`]
///   * [`io::Error`] => [`ErrorKind::Io`]
///   * `Box<dyn std::error::Error + Send` => [`ErrorKind::Custom`]
#[derive(Debug)]
#[non_exhaustive]
pub enum ErrorKind<'v> {
    /// The value's length, in bytes, was outside the range `[min, max]`.
    InvalidLength {
        /// The minimum length required, inclusive.
        min: Option<u64>,
        /// The maximum length required, inclusize.
        max: Option<u64>,
    },
    /// The value wasn't one of the valid `choices`.
    InvalidChoice {
        /// The choices that were expected.
        choices: Cow<'v, [Cow<'v, str>]>,
    },
    /// The integer value was outside the range `[start, end]`.
    OutOfRange {
        /// The start of the acceptable range, inclusive.
        start: Option<isize>,
        /// The end of the acceptable range, inclusive.
        end: Option<isize>,
    },
    /// A custom validation routine failed with message `.0`.
    Validation(Cow<'v, str>),
    /// One entity was expected but more than one was received.
    Duplicate,
    /// An entity was expected but was not received.
    Missing,
    /// An unexpected entity was received.
    Unexpected,
    /// An unknown entity was received.
    Unknown,
    /// A custom error occurred.
    Custom(Box<dyn std::error::Error + Send>),
    /// An error while parsing a multipart form occurred.
    Multipart(multer::Error),
    /// A string was invalid UTF-8.
    Utf8(Utf8Error),
    /// A value failed to parse as an integer.
    Int(ParseIntError),
    /// A value failed to parse as a boolean.
    Bool(ParseBoolError),
    /// A value failed to parse as a float.
    Float(ParseFloatError),
    /// A value failed to parse as an IP or socket address.
    Addr(AddrParseError),
    /// An I/O error occurred.
    Io(io::Error),
}

/// The erranous form entity or form component.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Entity {
    /// The form itself.
    Form,
    /// A field.
    Field,
    /// A [`ValueField`](crate::form::ValueField).
    ValueField,
    /// A [`DataField`](crate::form::DataField).
    DataField,
    /// A field name.
    Name,
    /// A field value.
    Value,
    /// A field name key.
    Key,
    /// A field name key index at index `.0`.
    Index(usize),
}

impl<'v> Errors<'v> {
    /// Create an empty collection of errors.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::Errors;
    ///
    /// let errors = Errors::new();
    /// assert!(errors.is_empty());
    /// ```
    pub fn new() -> Self {
        Errors(vec![])
    }

    /// Consumes `self` and returns a new `Errors` with each field name set to
    /// `name` if it was not already set.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Errors, ErrorKind};
    ///
    /// let mut errors = Errors::from(ErrorKind::Missing);
    /// assert!(errors[0].name.is_none());
    ///
    /// let mut errors = errors.with_name("foo");
    /// assert_eq!(errors[0].name.as_ref().unwrap(), "foo");
    ///
    /// errors.push(ErrorKind::Duplicate.into());
    /// let errors = errors.with_name("bar");
    /// assert_eq!(errors[0].name.as_ref().unwrap(), "foo");
    /// assert_eq!(errors[1].name.as_ref().unwrap(), "bar");
    /// ```
    pub fn with_name<N: Into<NameBuf<'v>>>(mut self, name: N) -> Self {
        self.set_name(name);
        self
    }

    /// Set the field name of each error in `self` to `name` if it is not
    /// already set.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Errors, ErrorKind};
    ///
    /// let mut errors = Errors::from(ErrorKind::Missing);
    /// assert!(errors[0].name.is_none());
    ///
    /// errors.set_name("foo");
    /// assert_eq!(errors[0].name.as_ref().unwrap(), "foo");
    ///
    /// errors.push(ErrorKind::Duplicate.into());
    /// let errors = errors.with_name("bar");
    /// assert_eq!(errors[0].name.as_ref().unwrap(), "foo");
    /// assert_eq!(errors[1].name.as_ref().unwrap(), "bar");
    /// ```
    pub fn set_name<N: Into<NameBuf<'v>>>(&mut self, name: N) {
        let name = name.into();
        for error in self.iter_mut() {
            if error.name.is_none() {
                error.set_name(name.clone());
            }
        }
    }

    /// Consumes `self` and returns a new `Errors` with each field value set to
    /// `value` if it was not already set.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Errors, ErrorKind};
    ///
    /// let mut errors = Errors::from(ErrorKind::Missing);
    /// assert!(errors[0].value.is_none());
    ///
    /// let mut errors = errors.with_value("foo");
    /// assert_eq!(errors[0].value.as_ref().unwrap(), "foo");
    ///
    /// errors.push(ErrorKind::Duplicate.into());
    /// let errors = errors.with_value("bar");
    /// assert_eq!(errors[0].value.as_ref().unwrap(), "foo");
    /// assert_eq!(errors[1].value.as_ref().unwrap(), "bar");
    /// ```
    pub fn with_value(mut self, value: &'v str) -> Self {
        self.set_value(value);
        self
    }

    /// Set the field value of each error in `self` to `value` if it is not
    /// already set.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Errors, ErrorKind};
    ///
    /// let mut errors = Errors::from(ErrorKind::Missing);
    /// assert!(errors[0].value.is_none());
    ///
    /// errors.set_value("foo");
    /// assert_eq!(errors[0].value.as_ref().unwrap(), "foo");
    ///
    /// errors.push(ErrorKind::Duplicate.into());
    /// let errors = errors.with_value("bar");
    /// assert_eq!(errors[0].value.as_ref().unwrap(), "foo");
    /// assert_eq!(errors[1].value.as_ref().unwrap(), "bar");
    /// ```
    pub fn set_value(&mut self, value: &'v str) {
        self.iter_mut().for_each(|e| e.set_value(value));
    }

    /// Returns the highest [`Error::status()`] of all of the errors in `self`
    /// or [`Status::InternalServerError`] if `self` is empty. This is the
    /// status that is set by the [`Form`](crate::form::Form) data guard on
    /// failure.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Error, Errors, ErrorKind};
    /// use rocket::http::Status;
    ///
    /// let mut errors = Errors::new();
    /// assert_eq!(errors.status(), Status::InternalServerError);
    ///
    /// errors.push(Error::from((None, Some(10u64))));
    /// assert_eq!(errors.status(), Status::PayloadTooLarge);
    ///
    /// errors.push(Error::from(ErrorKind::Missing));
    /// assert_eq!(errors.status(), Status::UnprocessableEntity);
    /// ```
    pub fn status(&self) -> Status {
        let max = self.iter().map(|e| e.status()).max();
        max.unwrap_or(Status::InternalServerError)
    }
}

impl crate::http::ext::IntoOwned for Errors<'_> {
    type Owned = Errors<'static>;

    fn into_owned(self) -> Self::Owned {
        Errors(self.0.into_owned())
    }
}

impl fmt::Display for Errors<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.len() == 1 {
            write!(f, "1 error:")?;
        } else {
            write!(f, "{} errors:", self.len())?;
        }
        for error in self.iter() {
            write!(f, "\n{}", error)?;
        }

        Ok(())
    }
}

impl<'v> std::ops::Deref for Errors<'v> {
    type Target = Vec<Error<'v>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'v> std::ops::DerefMut for Errors<'v> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'v, T: Into<Error<'v>>> From<T> for Errors<'v> {
    #[inline(always)]
    fn from(e: T) -> Self {
        Errors(vec![e.into()])
    }
}

impl<'v> From<Vec<Error<'v>>> for Errors<'v> {
    #[inline(always)]
    fn from(v: Vec<Error<'v>>) -> Self {
        Errors(v)
    }
}

impl<'v> IntoIterator for Errors<'v> {
    type Item = Error<'v>;

    type IntoIter = <Vec<Error<'v>> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'v> Error<'v> {
    /// Creates a new `Error` with `ErrorKind::Custom`.
    ///
    /// For validation errors, use [`Error::validation()`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::Error;
    ///
    /// fn from_fmt(error: std::fmt::Error) -> Error<'static> {
    ///     Error::custom(error)
    /// }
    /// ```
    pub fn custom<E>(error: E) -> Self
        where E: std::error::Error + Send + 'static
    {
        (Box::new(error) as Box<dyn std::error::Error + Send>).into()
    }

    /// Creates a new `Error` with `ErrorKind::Validation` and message `msg`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Error, ErrorKind, Entity};
    ///
    /// let error = Error::validation("invalid foo: need bar");
    /// assert!(matches!(error.kind, ErrorKind::Validation(_)));
    /// assert_eq!(error.entity, Entity::Value);
    /// ```
    pub fn validation<S: Into<Cow<'v, str>>>(msg: S) -> Self {
        ErrorKind::Validation(msg.into()).into()
    }

    /// Consumes `self` and returns a new `Error` with the entity set to
    /// `entity`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Error, ErrorKind, Entity};
    ///
    /// let error = Error::from(ErrorKind::Missing);
    /// assert_eq!(error.entity, Entity::Field);
    ///
    /// let error = error.with_entity(Entity::Key);
    /// assert_eq!(error.entity, Entity::Key);
    /// ```
    pub fn with_entity(mut self, entity: Entity) -> Self {
        self.set_entity(entity);
        self
    }

    /// Sets the error's entity to `entity.`
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Error, ErrorKind, Entity};
    ///
    /// let mut error = Error::from(ErrorKind::Missing);
    /// assert_eq!(error.entity, Entity::Field);
    ///
    /// error.set_entity(Entity::Key);
    /// assert_eq!(error.entity, Entity::Key);
    /// ```
    pub fn set_entity(&mut self, entity: Entity) {
        self.entity = entity;
    }

    /// Consumes `self` and returns a new `Error` with the field name set to
    /// `name` if it was not already set.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Error, ErrorKind};
    ///
    /// let error = Error::from(ErrorKind::Missing);
    /// assert!(error.name.is_none());
    ///
    /// let error = error.with_name("foo");
    /// assert_eq!(error.name.as_ref().unwrap(), "foo");
    ///
    /// let error = error.with_name("bar");
    /// assert_eq!(error.name.as_ref().unwrap(), "foo");
    /// ```
    pub fn with_name<N: Into<NameBuf<'v>>>(mut self, name: N) -> Self {
        self.set_name(name);
        self
    }

    /// Sets the field name of `self` to `name` if it is not already set.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Error, ErrorKind};
    ///
    /// let mut error = Error::from(ErrorKind::Missing);
    /// assert!(error.name.is_none());
    ///
    /// error.set_name("foo");
    /// assert_eq!(error.name.as_ref().unwrap(), "foo");
    ///
    /// let error = error.with_name("bar");
    /// assert_eq!(error.name.as_ref().unwrap(), "foo");
    /// ```
    pub fn set_name<N: Into<NameBuf<'v>>>(&mut self, name: N) {
        if self.name.is_none() {
            self.name = Some(name.into());
        }
    }

    /// Consumes `self` and returns a new `Error` with the value set to `value`
    /// if it was not already set.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Error, ErrorKind};
    ///
    /// let error = Error::from(ErrorKind::Missing);
    /// assert!(error.value.is_none());
    ///
    /// let error = error.with_value("foo");
    /// assert_eq!(error.value.as_ref().unwrap(), "foo");
    ///
    /// let error = error.with_value("bar");
    /// assert_eq!(error.value.as_ref().unwrap(), "foo");
    /// ```
    pub fn with_value(mut self, value: &'v str) -> Self {
        self.set_value(value);
        self
    }

    /// Set the field value of `self` to `value` if it is not already set.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::error::{Error, ErrorKind};
    ///
    /// let mut error = Error::from(ErrorKind::Missing);
    /// assert!(error.value.is_none());
    ///
    /// error.set_value("foo");
    /// assert_eq!(error.value.as_ref().unwrap(), "foo");
    ///
    /// error.set_value("bar");
    /// assert_eq!(error.value.as_ref().unwrap(), "foo");
    /// ```
    pub fn set_value(&mut self, value: &'v str) {
        if self.value.is_none() {
            self.value = Some(value.into());
        }
    }

    /// Returns `true` if this error applies to a field named `name`. **This is
    /// _different_ than simply comparing `name`.**
    ///
    /// Unlike [`Error::is_for_exactly()`], this method returns `true` if the
    /// error's field name is a **prefix of `name`**. This is typically what is
    /// desired as errors apply to a field and its children: `a.b` applies to
    /// the nested fields `a.b.c`, `a.b.d` and so on.
    ///
    /// Returns `false` if `self` has no field name.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::Error;
    ///
    /// // returns `false` without a field name
    /// let error = Error::validation("bad `foo`");
    /// assert!(!error.is_for_exactly("a.b"));
    ///
    /// // `a.b` is a prefix all of these field names
    /// let error = error.with_name("a.b");
    /// assert!(error.is_for("a.b"));
    /// assert!(error.is_for("a[b]"));
    /// assert!(error.is_for("a.b.c"));
    /// assert!(error.is_for("a.b[c]"));
    /// assert!(error.is_for("a.b.c[d]"));
    /// assert!(error.is_for("a.b.c.d.foo"));
    ///
    /// // ...but not of these.
    /// assert!(!error.is_for("a.c"));
    /// assert!(!error.is_for("a"));
    /// ```
    pub fn is_for<N: AsRef<Name>>(&self, name: N) -> bool {
        self.name.as_ref().map(|e_name| {
            if e_name.is_empty() != name.as_ref().is_empty() {
                return false;
            }

            let mut e_keys = e_name.keys();
            let mut n_keys = name.as_ref().keys();
            loop {
                match (e_keys.next(), n_keys.next()) {
                    (Some(e), Some(n)) if e == n => continue,
                    (Some(_), Some(_)) => return false,
                    (Some(_), None) => return false,
                    (None, _) => break,
                }
            }

            true
        })
        .unwrap_or(false)
    }

    /// Returns `true` if this error applies to exactly the field named `name`.
    /// Returns `false` if `self` has no field name.
    ///
    /// Unlike [`Error::is_for()`], this method returns `true` only when the
    /// error's field name is exactly `name`. This is _not_ typically what is
    /// desired.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::Error;
    ///
    /// // returns `false` without a field name
    /// let error = Error::validation("bad `foo`");
    /// assert!(!error.is_for_exactly("a.b"));
    ///
    /// let error = error.with_name("a.b");
    /// assert!(error.is_for_exactly("a.b"));
    /// assert!(error.is_for_exactly("a[b]"));
    ///
    /// // does not return `true` when the name is a prefix
    /// assert!(!error.is_for_exactly("a.b.c"));
    /// assert!(!error.is_for_exactly("a.b[c]"));
    /// assert!(!error.is_for_exactly("a.b.c[d]"));
    /// assert!(!error.is_for_exactly("a.b.c.d.foo"));
    ///
    /// // does not return `true` when the name is different
    /// assert!(!error.is_for("a.c"));
    /// assert!(!error.is_for("a"));
    /// ```
    pub fn is_for_exactly<N: AsRef<Name>>(&self, name: N) -> bool {
        self.name.as_ref()
            .map(|n| name.as_ref() == n)
            .unwrap_or(false)
    }

    /// Returns the most reasonable `Status` associated with this error. These
    /// are:
    ///
    ///  * **`PayloadTooLarge`** if the error kind is:
    ///    - `InvalidLength` with min of `None`
    ///    - `Multpart(FieldSizeExceeded | StreamSizeExceeded)`
    ///  * **`InternalServerError`** if the error kind is:
    ///    - `Unknown`
    ///  * **`BadRequest`** if the error kind is:
    ///    - `Io` with an `entity` of `Form`
    ///  * **`UnprocessableEntity`** otherwise
    ///
    /// # Example
    ///
    ///  ```rust
    ///  use rocket::form::error::{Error, ErrorKind, Entity};
    ///  use rocket::http::Status;
    ///
    ///  let error = Error::validation("bad `foo`");
    ///  assert_eq!(error.status(), Status::UnprocessableEntity);
    ///
    ///  let error = Error::from((None, Some(10u64)));
    ///  assert_eq!(error.status(), Status::PayloadTooLarge);
    ///
    ///  let error = Error::from(ErrorKind::Unknown);
    ///  assert_eq!(error.status(), Status::InternalServerError);
    ///
    ///  // default entity for `io::Error` is `Form`.
    ///  let error = Error::from(std::io::Error::last_os_error());
    ///  assert_eq!(error.status(), Status::BadRequest);
    ///
    ///  let error = error.with_entity(Entity::Value);
    ///  assert_eq!(error.status(), Status::UnprocessableEntity);
    ///  ```
    pub fn status(&self) -> Status {
        use ErrorKind::*;
        use multer::Error::*;

        match self.kind {
            InvalidLength { min: None, .. }
            | Multipart(FieldSizeExceeded { .. })
            | Multipart(StreamSizeExceeded { .. }) => Status::PayloadTooLarge,
            Unknown => Status::InternalServerError,
            Io(_) | _ if self.entity == Entity::Form => Status::BadRequest,
            _ => Status::UnprocessableEntity
        }
    }
}

impl<'v> Serialize for Error<'v> {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let mut err = ser.serialize_struct("Error", 3)?;
        err.serialize_field("name", &self.name)?;
        err.serialize_field("value", &self.value)?;
        err.serialize_field("entity", &self.entity.to_string())?;
        err.serialize_field("msg", &self.to_string())?;
        err.end()
    }
}

impl crate::http::ext::IntoOwned for Error<'_> {
    type Owned = Error<'static>;

    fn into_owned(self) -> Self::Owned {
        Error {
            name: self.name.into_owned(),
            value: self.value.into_owned(),
            kind: self.kind.into_owned(),
            entity: self.entity,
        }
    }
}

impl<'v> std::ops::Deref for Error<'v> {
    type Target = ErrorKind<'v>;

    fn deref(&self) -> &Self::Target {
        &self.kind
    }
}

impl fmt::Display for Error<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)
    }
}

impl<'v, T: Into<ErrorKind<'v>>> From<T> for Error<'v> {
    #[inline(always)]
    fn from(k: T) -> Self {
        let kind = k.into();
        let entity = Entity::default_for(&kind);
        Error { name: None, value: None, kind, entity }
    }
}

impl<'a> From<multer::Error> for Error<'a> {
    fn from(error: multer::Error) -> Self {
        use multer::Error::*;
        use self::ErrorKind::*;

        let incomplete = Error::from(InvalidLength { min: None, max: None });
        match error {
            UnknownField { field_name: Some(name) } => Error::from(Unexpected).with_name(name),
            UnknownField { field_name: None } => Error::from(Unexpected),
            FieldSizeExceeded { limit, field_name } => {
                let e = Error::from((None, Some(limit)));
                match field_name {
                    Some(name) => e.with_name(name),
                    None => e
                }
            },
            StreamSizeExceeded { limit } => {
                Error::from((None, Some(limit))).with_entity(Entity::Form)
            }
            IncompleteFieldData { field_name: Some(name) } => incomplete.with_name(name),
            IncompleteFieldData { field_name: None } => incomplete,
            IncompleteStream | IncompleteHeaders => incomplete.with_entity(Entity::Form),
            e => Error::from(ErrorKind::Multipart(e))
        }
    }
}

impl fmt::Display for ErrorKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::InvalidLength { min, max } => {
                match (min, max) {
                    (None, None) => write!(f, "invalid length: incomplete")?,
                    (None, Some(k)) if *k < 1024 => write!(f, "length cannot exceed {}", k)?,
                    (None, Some(k)) => write!(f, "size must not exceed {}", ByteUnit::from(*k))?,
                    (Some(1), None) => write!(f, "cannot be empty")?,
                    (Some(k), None) if *k < 1024 => write!(f, "expected at least {}", k)?,
                    (Some(k), None) => write!(f, "size must be at least {}", ByteUnit::from(*k))?,
                    (Some(i), Some(j)) if *i < 1024 && *j < 1024 => {
                        write!(f, "length must be between {} and {}", i, j)?;
                    }
                    (Some(i), Some(j)) => {
                        let (i, j) = (ByteUnit::from(*i), ByteUnit::from(*j));
                        write!(f, "size must be between {} and {}", i, j)?;
                    }
                }
            }
            ErrorKind::InvalidChoice { choices } => {
                match *choices.as_ref() {
                    [] => write!(f, "invalid choice")?,
                    [ref choice] => write!(f, "expected {}", choice)?,
                    _ => {
                        write!(f, "expected one of ")?;
                        for (i, choice) in choices.iter().enumerate() {
                            if i != 0 { write!(f, ", ")?; }
                            write!(f, "`{}`", choice)?;
                        }
                    }
                }
            }
            ErrorKind::OutOfRange { start, end } => {
                match (start, end) {
                    (None, None) => write!(f, "value is out of range")?,
                    (None, Some(k)) => write!(f, "value cannot exceed {}", k)?,
                    (Some(k), None) => write!(f, "value must be at least {}", k)?,
                    (Some(i), Some(j)) => write!(f, "value must be between {} and {}", i, j)?,
                }
            }
            ErrorKind::Validation(msg) => msg.fmt(f)?,
            ErrorKind::Duplicate => "duplicate".fmt(f)?,
            ErrorKind::Missing => "missing".fmt(f)?,
            ErrorKind::Unexpected => "unexpected".fmt(f)?,
            ErrorKind::Unknown => "unknown internal error".fmt(f)?,
            ErrorKind::Custom(e) => e.fmt(f)?,
            ErrorKind::Multipart(e) => write!(f, "invalid multipart: {}", e)?,
            ErrorKind::Utf8(e) => write!(f, "invalid UTF-8: {}", e)?,
            ErrorKind::Int(e) => write!(f, "invalid integer: {}", e)?,
            ErrorKind::Bool(e) => write!(f, "invalid boolean: {}", e)?,
            ErrorKind::Float(e) => write!(f, "invalid float: {}", e)?,
            ErrorKind::Addr(e) => write!(f, "invalid address: {}", e)?,
            ErrorKind::Io(e) => write!(f, "i/o error: {}", e)?,
        }

        Ok(())
    }
}

impl crate::http::ext::IntoOwned for ErrorKind<'_> {
    type Owned = ErrorKind<'static>;

    fn into_owned(self) -> Self::Owned {
        use ErrorKind::*;

        match self {
            InvalidLength { min, max } => InvalidLength { min, max },
            OutOfRange { start, end } => OutOfRange { start, end },
            Validation(s) => Validation(s.into_owned().into()),
            Duplicate => Duplicate,
            Missing => Missing,
            Unexpected => Unexpected,
            Unknown => Unknown,
            Custom(e) => Custom(e),
            Multipart(e) => Multipart(e),
            Utf8(e) => Utf8(e),
            Int(e) => Int(e),
            Bool(e) => Bool(e),
            Float(e) => Float(e),
            Addr(e) => Addr(e),
            Io(e) => Io(e),
            InvalidChoice { choices } => InvalidChoice {
                choices: choices.iter()
                    .map(|s| Cow::Owned(s.to_string()))
                    .collect::<Vec<_>>()
                    .into()
            }
        }
    }
}


impl<'a, 'b> PartialEq<ErrorKind<'b>> for ErrorKind<'a> {
    fn eq(&self, other: &ErrorKind<'b>) -> bool {
        use ErrorKind::*;
        match (self, other) {
            (InvalidLength { min: a, max: b }, InvalidLength { min, max }) => min == a && max == b,
            (InvalidChoice { choices: a }, InvalidChoice { choices }) => choices == a,
            (OutOfRange { start: a, end: b }, OutOfRange { start, end }) => start == a && end == b,
            (Validation(a), Validation(b)) => a == b,
            (Duplicate, Duplicate) => true,
            (Missing, Missing) => true,
            (Unexpected, Unexpected) => true,
            (Custom(_), Custom(_)) => true,
            (Multipart(a), Multipart(b)) => a == b,
            (Utf8(a), Utf8(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (Bool(a), Bool(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (Addr(a), Addr(b)) => a == b,
            (Io(a), Io(b)) => a.kind() == b.kind(),
            _ => false,
        }
    }
}

impl From<(Option<u64>, Option<u64>)> for ErrorKind<'_> {
    fn from((min, max): (Option<u64>, Option<u64>)) -> Self {
        ErrorKind::InvalidLength { min, max }
    }
}

impl<'a, 'v: 'a> From<&'static [Cow<'v, str>]> for ErrorKind<'a> {
    fn from(choices: &'static [Cow<'v, str>]) -> Self {
        ErrorKind::InvalidChoice { choices: choices.into() }
    }
}

impl<'a, 'v: 'a> From<Vec<Cow<'v, str>>> for ErrorKind<'a> {
    fn from(choices: Vec<Cow<'v, str>>) -> Self {
        ErrorKind::InvalidChoice { choices: choices.into() }
    }
}

impl From<(Option<isize>, Option<isize>)> for ErrorKind<'_> {
    fn from((start, end): (Option<isize>, Option<isize>)) -> Self {
        ErrorKind::OutOfRange { start, end }
    }
}

impl From<(Option<ByteUnit>, Option<ByteUnit>)> for ErrorKind<'_> {
    fn from((start, end): (Option<ByteUnit>, Option<ByteUnit>)) -> Self {
        ErrorKind::from((start.map(ByteUnit::as_u64), end.map(ByteUnit::as_u64)))
    }
}

macro_rules! impl_from_choices {
    ($($size:literal),*) => ($(
        impl<'a, 'v: 'a> From<&'static [Cow<'v, str>; $size]> for ErrorKind<'a> {
            fn from(choices: &'static [Cow<'v, str>; $size]) -> Self {
                let choices = &choices[..];
                ErrorKind::InvalidChoice { choices: choices.into() }
            }
        }
    )*)
}

impl_from_choices!(1, 2, 3, 4, 5, 6, 7, 8);

macro_rules! impl_from_for {
    (<$l:lifetime> $T:ty => $V:ty as $variant:ident) => (
        impl<$l> From<$T> for $V {
            fn from(value: $T) -> Self {
                <$V>::$variant(value)
            }
        }
    )
}

impl_from_for!(<'a> Utf8Error => ErrorKind<'a> as Utf8);
impl_from_for!(<'a> ParseIntError => ErrorKind<'a> as Int);
impl_from_for!(<'a> ParseFloatError => ErrorKind<'a> as Float);
impl_from_for!(<'a> ParseBoolError => ErrorKind<'a> as Bool);
impl_from_for!(<'a> AddrParseError => ErrorKind<'a> as Addr);
impl_from_for!(<'a> io::Error => ErrorKind<'a> as Io);
impl_from_for!(<'a> Box<dyn std::error::Error + Send> => ErrorKind<'a> as Custom);

impl fmt::Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let string = match self {
            Entity::Form => "form",
            Entity::Field => "field",
            Entity::ValueField => "value field",
            Entity::DataField => "data field",
            Entity::Name => "name",
            Entity::Value => "value",
            Entity::Key => "key",
            Entity::Index(k) => return write!(f, "index {}", k),
        };

        string.fmt(f)
    }
}

impl Entity {
    /// The default entity for an [`Error`] created for `ErrorKind`.
    ///
    ///  * **[`Field`]** if `Duplicate`, `Missing`, `Unexpected`, or `Unknown`
    ///  * **[`Form`]** if `Multipart` or `Io`
    ///  * **[`Value`]** otherwise
    ///
    /// [`Field`]: Entity::Field
    /// [`Form`]: Entity::Form
    /// [`Value`]: Entity::Value
    pub const fn default_for(kind: &ErrorKind<'_>) -> Self {
        match kind {
            | ErrorKind::InvalidLength { .. }
            | ErrorKind::InvalidChoice { .. }
            | ErrorKind::OutOfRange { .. }
            | ErrorKind::Validation { .. }
            | ErrorKind::Utf8(_)
            | ErrorKind::Int(_)
            | ErrorKind::Float(_)
            | ErrorKind::Bool(_)
            | ErrorKind::Custom(_)
            | ErrorKind::Addr(_) => Entity::Value,

            | ErrorKind::Duplicate
            | ErrorKind::Missing
            | ErrorKind::Unknown
            | ErrorKind::Unexpected => Entity::Field,

            | ErrorKind::Multipart(_)
            | ErrorKind::Io(_) => Entity::Form,
        }
    }
}
