//! Local form testing utilities.
//!
//! This module contains `LocalForm` for use when testing form handling in
//! Rocket.

use std::io::Write;
use std::borrow::Cow;
use std::ops::Deref;

use crate::http::{ContentType, RawStr};
use crate::form::{ValueField, name::NameBuf};
use crate::http::uri::fmt::{UriDisplay, Query};

/// A struct that can be used to easily create simple and multipart form data
/// for testing purposes.
///
/// ## Example
///
/// The following snippet creates a request with a form submission:
///
/// ```rust
/// # use rocket::launch;
/// use rocket::local::form::LocalForm;
/// use rocket::local::blocking::{Client, LocalRequest};
/// use rocket::http::{ContentType};
///
/// #[launch]
/// fn rocket() -> _ {
///     rocket::build()
///     #    .configure(rocket::Config::debug_default())
/// }
///
/// let client = Client::tracked(rocket()).expect("valid `Rocket`");
/// let req = client.post("/")
///     .form(LocalForm::new()
///             .field("field", "value")
///             .file("foo.txt", ContentType::Plain, "hi there!"));
///
/// let response = req.dispatch();
/// ```
pub struct LocalForm<'v>(Vec<LocalField<'v>>);

#[derive(Debug, PartialEq)]
pub(crate) enum LocalField<'v> {
    Value(NameBuf<'v>, String),
    ValueEncoded(Cow<'v, RawStr>, Cow<'v, RawStr>),
    Data {
        name: NameBuf<'v>,
        file_name: Option<&'v str>,
        content_type: ContentType,
        data: Vec<u8>,
    },
}

impl<'v> LocalForm<'v> {
    /// Creates a new `LocalForm` that can have fields and values added to.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// A percent-decoded `name` and `value`.
    pub fn field<N, V>(mut self, name: N, value: V) -> Self
    where
        N: Into<NameBuf<'v>>, V: Into<String> + UriDisplay<Query>,
    {
        self.add_field(name.into(), value.into());
        self
    }

    /// Adds all the `fields` to `self`.
    pub fn fields<I, F>(mut self, fields: I) -> Self
    where
        I: Iterator<Item = F>, F: Into<ValueField<'v>>,
    {
        fields.for_each(|f| {
            let field = f.into();
            self.add_field(field.name, field.value);
        });
        self
    }

    /// A percent-encoded `name` and `value`.
    pub fn raw_field(
        mut self,
        name: impl Into<Cow<'v, RawStr>>,
        value: impl Into<Cow<'v, RawStr>>,
    ) -> Self {
        let name = name.into();
        let value = value.into();

        let name = Self::encode_if_not_encoded(&name);
        let value = Self::encode_if_not_encoded(&value);

        self.0.push(LocalField::ValueEncoded(name, value));
        self
    }

    /// Adds a field for the file with the name `file_name`, content-type `ct`, and
    /// file contents `data`.
    pub fn file<N, V>(mut self, file_name: N, ct: ContentType, data: V) -> Self
    where
        N: Into<Option<&'v str>>, V: AsRef<[u8]>,
    {
        self.0.push(LocalField::Data {
            name: NameBuf::from("file"),
            file_name: file_name.into(),
            content_type: ct,
            data: data.as_ref().into(),
        });
        self
    }

    /// Add a data field with a content-type `ct` and binary `data`.
    pub fn data<V>(mut self, ct: ContentType, data: V) -> Self
    where
        V: AsRef<[u8]>,
    {
        self.0.push(LocalField::Data {
            name: NameBuf::from("file"),
            file_name: None,
            content_type: ct,
            data: data.as_ref().into(),
        });
        self
    }

    /// The full content-type for this form.
    pub fn content_type(&self) -> ContentType {
        if self.0.iter().any(|field| matches!(field, LocalField::Data{..})) {
            return ContentType::with_params("multipart", "form-data", ("boundary", "X-BOUNDARY"));
        }

        ContentType::Form
    }

    /// The full body data for this form.
    pub fn body_data(&self) -> Vec<u8> {
        let result = if self.0.iter().any(|field| matches!(field, LocalField::Data{..})) {
            self.format_multipart()
        } else {
            self.format_simple()
        };

        result.unwrap_or(Vec::new())
    }

    fn add_field<N, V>(&mut self, name: N, value: V)
        where N: Into<NameBuf<'v>>, V: Into<String>
    {
        self.0.push(LocalField::Value(
            name.into(),
            value.into(),
        ));
    }

    pub(crate) fn encode_if_not_encoded(value: &RawStr) -> Cow<'static, RawStr> {
        let decoded = value.percent_decode();
        if let Ok(decoded) = decoded {
            let reencoded = RawStr::new(&*decoded).percent_encode();
            if reencoded.deref() == value {
                return reencoded.into_owned().into();
            }
        }

        value.percent_encode().into_owned().into()
    }

    fn format_simple(&self) -> Result<Vec<u8>, std::io::Error> {
        let mut buf: Vec<u8> = Vec::new();

        let mut iter = self.0.iter();
        match iter.next() {
            Some(LocalField::Value(name, value)) => write!(
                &mut buf,
                "{}={}", RawStr::new(&format!("{}", name)).percent_encode(),
                &value as &dyn UriDisplay<Query>,
            )?,
            Some(LocalField::ValueEncoded(name, value)) => write!(&mut buf, "{}={}", name, value)?,
            _ => return Ok(buf)
        };

        for field in iter {
            match field {
                LocalField::Value(name, value) => {
                    write!(
                        &mut buf,
                        "&{}={}", RawStr::new(&format!("{}", name)).percent_encode(),
                        &value as &dyn UriDisplay<Query>,
                    )?;
                },
                LocalField::ValueEncoded(name, value) => {
                    write!(&mut buf, "&{}={}", name, value)?;
                },
                _ => {},
            }
        }
        Ok(buf)
    }

    fn format_multipart(&self) -> Result<Vec<u8>, std::io::Error> {
        let mut buf: Vec<u8> = Vec::new();
        for field in self.0.iter() {
            match field {
                LocalField::ValueEncoded(name, value) => {
                    write!(&mut buf, "--X-BOUNDARY\r\n")?;
                    write!(&mut buf, "Content-Disposition: form-data; name=\"{}\"\r\n", name)?;
                    write!(&mut buf, "\r\n")?;
                    write!(&mut buf, "{}\r\n", value)?;
                }
                // TODO: URL encode the name and value
                LocalField::Value(name, value) => {
                    write!(&mut buf, "--X-BOUNDARY\r\n")?;
                    write!(&mut buf, "Content-Disposition: form-data; name=\"{}\"\r\n", name)?;
                    write!(&mut buf, "\r\n")?;
                    write!(&mut buf, "{}\r\n", &value as &dyn UriDisplay<Query>)?;
                },
                LocalField::Data{ name, file_name, content_type, data } => {
                    write!(&mut buf, "--X-BOUNDARY\r\n")?;
                    write!(
                        &mut buf,
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
                        name,
                        file_name.unwrap_or(""),
                    )?;
                    write!(&mut buf, "Content-Type: {}\r\n", content_type)?;
                    write!(&mut buf, "\r\n")?;
                    buf.extend(data);
                    write!(&mut buf, "\r\n")?;
                },
            }
        }
        write!(&mut buf, "--X-BOUNDARY--\r\n")?;
        write!(&mut buf, "")?;
        Ok(buf)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_content_type() {
        let form = LocalForm::new()
            .field("name[]", "john doe");
        assert_eq!(ContentType::Form, form.content_type());

        let form = form.file("foo.txt", ContentType::Plain, "123");
        assert_eq!(ContentType::FormData, form.content_type());
    }

    #[test]
    fn test_body_data() {
        let simple_body = b"field=value&is%20it=a%20cat%3F";
        let form = LocalForm::new()
            .field("field", "value")
            .field("is it", "a cat?");

        assert_eq!(
            &form.body_data(),
            simple_body,
        );

        let multipart_body = &[
            "--X-BOUNDARY",
            r#"Content-Disposition: form-data; name="names[]""#,
            "",
            "abcd",
            "--X-BOUNDARY",
            r#"Content-Disposition: form-data; name="names[]""#,
            "",
            "123",
            "--X-BOUNDARY",
            r#"Content-Disposition: form-data; name="file"; filename="foo.txt""#,
            "Content-Type: text/plain; charset=utf-8",
            "",
            "hi there",
            "--X-BOUNDARY--",
            "",
        ].join("\r\n");
        let form = LocalForm::new()
            .field("names[]", "abcd")
            .field("names[]", "123")
            .file("foo.txt", ContentType::Plain, "hi there");

        assert_eq!(multipart_body.as_str(), String::from_utf8_lossy(&form.body_data()));
    }

    #[test]
    fn test_encoding_detection() {
        let unencoded = Cow::Borrowed(RawStr::new("is this encoded?"));
        let encoded = Cow::Borrowed(RawStr::new("is%20this%20encoded%3F"));
        assert!(LocalForm::encode_if_not_encoded(&unencoded) != unencoded);
        assert_eq!(LocalForm::encode_if_not_encoded(&encoded), encoded);
    }

    #[test]
    fn test_raw_field() {
        let simple_body = b"is%20it=a%20cat%3F";
        let form = LocalForm::new()
            .raw_field(RawStr::new("is%20it"), RawStr::new("a%20cat%3F"));

        assert_eq!(&form.body_data(), simple_body);
    }
}
