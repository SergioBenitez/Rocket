use crate::form::{name::NameView, error::{Error, ErrorKind, Entity}};
use crate::http::{ContentType, RawStr};
use crate::{Request, Data};
use crate::fs::FileName;

/// A form field with a string value.
///
/// Rocket preprocesses all form fields into either [`ValueField`]s or
/// [`DataField`]s. All fields from url-encoded forms, and fields without
/// Content-Types from multipart forms, are preprocessed as a `ValueField`.
#[derive(Debug, Clone)]
pub struct ValueField<'r> {
    /// The (decoded) name of the form field.
    pub name: NameView<'r>,
    /// The (decoded) value of the form field.
    pub value: &'r str,
}

/// A multipart form field with an underlying data stream.
///
/// Rocket preprocesses all form fields into either [`ValueField`]s or
/// [`DataField`]s. Multipart form fields with a `Content-Type` are preprocessed
/// as a `DataField`. The underlying data is _not_ read into memory, but
/// instead, streamable from the contained [`Data`] structure.
pub struct DataField<'r, 'i> {
    /// The (decoded) name of the form field.
    pub name: NameView<'r>,
    /// The form fields's file name.
    pub file_name: Option<&'r FileName>,
    /// The form field's Content-Type, as submitted, which may or may not
    /// reflect on `data`.
    pub content_type: ContentType,
    /// The request in which the form field was submitted.
    pub request: &'r Request<'i>,
    /// The raw data stream.
    pub data: Data<'r>,
}

impl<'v> ValueField<'v> {
    /// Parse a field string, where both the key and value are assumed to be
    /// URL-decoded while preserving the `=` delimiter, into a `ValueField`.
    ///
    /// This implements 3.2, 3.3 of [section 5.1 of the WHATWG living standard].
    ///
    /// [section 5.1 of the WHATWG living standard]: https://url.spec.whatwg.org/#urlencoded-parsing
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::ValueField;
    ///
    /// let parsed = ValueField::parse("a cat=an A+ pet");
    /// assert_eq!(parsed.name, "a cat");
    /// assert_eq!(parsed.value, "an A+ pet");
    ///
    /// let parsed = ValueField::parse("a cat is an A+ pet");
    /// assert_eq!(parsed.name, "a cat is an A+ pet");
    /// assert_eq!(parsed.value, "");
    ///
    /// let parsed = ValueField::parse("cat.food=yum?");
    /// assert_eq!(parsed.name, "cat");
    /// assert_eq!(parsed.name.source(), "cat.food");
    /// assert_eq!(parsed.value, "yum?");
    /// ```
    pub fn parse(field: &'v str) -> Self {
        // WHATWG URL Living Standard 5.1 steps 3.2, 3.3.
        let (name, val) = RawStr::new(field).split_at_byte(b'=');
        ValueField::from((name.as_str(), val.as_str()))
    }

    /// Create a `ValueField` from a value, which is assumed to be URL-decoded.
    /// The field `name` will be empty.
    ///
    /// This is equivalent to `ValueField::from(("", value))`. To create a
    /// `ValueField` from both a `name` and a `value`, use
    /// `ValueField::from((name, value))`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::ValueField;
    ///
    /// let parsed = ValueField::from_value("A+=kitten");
    /// assert_eq!(parsed.name, "");
    /// assert_eq!(parsed.value, "A+=kitten");
    /// ```
    pub fn from_value(value: &'v str) -> Self {
        ValueField::from(("", value))
    }

    /// Shift the `name` of `self` and return `self` with the shfited `name`.
    ///
    /// See [`NameView::shift()`] for the details on name "shifting".
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::ValueField;
    ///
    /// let parsed = ValueField::parse("cat.food=yum?");
    /// assert_eq!(parsed.name, "cat");
    /// assert_eq!(parsed.name.source(), "cat.food");
    /// assert_eq!(parsed.name.key_lossy(), "cat");
    ///
    /// let shifted = parsed.shift();
    /// assert_eq!(shifted.name, "cat.food");
    /// assert_eq!(shifted.name.key_lossy(), "food");
    /// ```
    pub fn shift(mut self) -> Self {
        self.name.shift();
        self
    }

    /// Creates a complete unexpected value field [`Error`] from `self`.
    ///
    /// The error will have the following properties:
    ///   * `kind`: [`ErrorKind::Unexpected`]
    ///   * `name`: [`self.name.source()`](NameView::source())
    ///   * `value`: [`self.value`](ValueField::value)
    ///   * `entity`: [`Entity::ValueField`]
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::ValueField;
    /// use rocket::form::error::{ErrorKind, Entity};
    ///
    /// let field = ValueField::parse("cat.food=yum?");
    /// let error = field.unexpected();
    ///
    /// assert_eq!(error.name.as_ref().unwrap(), "cat.food");
    /// assert_eq!(error.value.as_ref().unwrap(), "yum?");
    /// assert_eq!(error.kind, ErrorKind::Unexpected);
    /// assert_eq!(error.entity, Entity::ValueField);
    /// ```
    pub fn unexpected(&self) -> Error<'v> {
        Error::from(ErrorKind::Unexpected)
            .with_name(self.name.source())
            .with_value(self.value)
            .with_entity(Entity::ValueField)
    }

    /// Creates a complete mising value field [`Error`] from `self`.
    ///
    /// The error will have the following properties:
    ///   * `kind`: [`ErrorKind::Missing`]
    ///   * `name`: [`self.name.source()`](NameView::source())
    ///   * `value`: [`self.value`](ValueField::value)
    ///   * `entity`: [`Entity::ValueField`]
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::ValueField;
    /// use rocket::form::error::{ErrorKind, Entity};
    ///
    /// let field = ValueField::parse("cat.food=yum?");
    /// let error = field.missing();
    ///
    /// assert_eq!(error.name.as_ref().unwrap(), "cat.food");
    /// assert_eq!(error.value.as_ref().unwrap(), "yum?");
    /// assert_eq!(error.kind, ErrorKind::Missing);
    /// assert_eq!(error.entity, Entity::ValueField);
    /// ```
    pub fn missing(&self) -> Error<'v> {
        Error::from(ErrorKind::Missing)
            .with_name(self.name.source())
            .with_value(self.value)
            .with_entity(Entity::ValueField)
    }
}

impl<'v> DataField<'v, '_> {
    /// Shift the `name` of `self` and return `self` with the shifted `name`.
    ///
    /// This is identical to [`ValueField::shift()`] but for `DataField`s. See
    /// [`NameView::shift()`] for the details on name "shifting".
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::DataField;
    ///
    /// fn push_data(field: DataField<'_, '_>) {
    ///     let shifted = field.shift();
    /// }
    /// ```
    pub fn shift(mut self) -> Self {
        self.name.shift();
        self
    }

    /// Creates a complete unexpected data field [`Error`] from `self`.
    ///
    /// The error will have the following properties:
    ///   * `kind`: [`ErrorKind::Unexpected`]
    ///   * `name`: [`self.name.source()`](NameView::source())
    ///   * `value`: `None`
    ///   * `entity`: [`Entity::DataField`]
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::DataField;
    ///
    /// fn push_data(field: DataField<'_, '_>) {
    ///     let error = field.unexpected();
    /// }
    /// ```
    pub fn unexpected(&self) -> Error<'v> {
        Error::from(ErrorKind::Unexpected)
            .with_name(self.name.source())
            .with_entity(Entity::DataField)
    }
}

impl<'a> From<(&'a str, &'a str)> for ValueField<'a> {
    fn from((name, value): (&'a str, &'a str)) -> Self {
        ValueField { name: NameView::new(name), value }
    }
}

impl<'a, 'b> PartialEq<ValueField<'b>> for ValueField<'a> {
    fn eq(&self, other: &ValueField<'b>) -> bool {
        self.name == other.name && self.value == other.value
    }
}
