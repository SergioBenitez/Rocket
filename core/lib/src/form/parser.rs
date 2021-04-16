use std::cell::UnsafeCell;

use multer::Multipart;
use parking_lot::{RawMutex, lock_api::RawMutex as _};
use either::Either;

use crate::request::{Request, local_cache};
use crate::data::{Data, Limits, Outcome};
use crate::form::prelude::*;
use crate::http::RawStr;

type Result<'r, T> = std::result::Result<T, Error<'r>>;

type Field<'r, 'i> = Either<ValueField<'r>, DataField<'r, 'i>>;

pub struct Buffer {
    strings: UnsafeCell<Vec<String>>,
    mutex: RawMutex,
}

pub struct MultipartParser<'r, 'i> {
    request: &'r Request<'i>,
    buffer: &'r Buffer,
    source: Multipart,
    done: bool,
}

pub struct RawStrParser<'r> {
    buffer: &'r Buffer,
    source: &'r RawStr,
}

pub enum Parser<'r, 'i> {
    Multipart(MultipartParser<'r, 'i>),
    RawStr(RawStrParser<'r>),
}

impl<'r, 'i> Parser<'r, 'i> {
    pub async fn new(req: &'r Request<'i>, data: Data) -> Outcome<Parser<'r, 'i>, Errors<'r>> {
        let parser = match req.content_type() {
            Some(c) if c.is_form() => Self::from_form(req, data).await,
            Some(c) if c.is_form_data() => Self::from_multipart(req, data).await,
            _ => return Outcome::Forward(data),
        };

        match parser {
            Ok(storage) => Outcome::Success(storage),
            Err(e) => Outcome::Failure((e.status(), e.into()))
        }
    }

    async fn from_form(req: &'r Request<'i>, data: Data) -> Result<'r, Parser<'r, 'i>> {
        let limit = req.limits().get("form").unwrap_or(Limits::FORM);
        let string = data.open(limit).into_string().await?;
        if !string.is_complete() {
            Err((None, Some(limit.as_u64())))?
        }

        Ok(Parser::RawStr(RawStrParser {
            buffer: local_cache!(req, Buffer::new()),
            source: RawStr::new(local_cache!(req, string.into_inner())),
        }))
    }

    async fn from_multipart(req: &'r Request<'i>, data: Data) -> Result<'r, Parser<'r, 'i>> {
        let boundary = req.content_type()
            .ok_or(multer::Error::NoMultipart)?
            .param("boundary")
            .ok_or(multer::Error::NoBoundary)?;

        let form_limit = req.limits()
            .get("data-form")
            .unwrap_or(Limits::DATA_FORM);

        Ok(Parser::Multipart(MultipartParser {
            request: req,
            buffer: local_cache!(req, Buffer::new()),
            source: Multipart::with_reader(data.open(form_limit), boundary),
            done: false,
        }))
    }

    pub async fn next(&mut self) -> Option<Result<'r, Field<'r, 'i>>> {
        match self {
            Parser::Multipart(ref mut p) => p.next().await,
            Parser::RawStr(ref mut p) => p.next().map(|f| Ok(Either::Left(f)))
        }
    }
}

impl<'r> RawStrParser<'r> {
    pub fn new(buffer: &'r Buffer, source: &'r RawStr) -> Self {
        RawStrParser { buffer, source }
    }
}

impl<'r> Iterator for RawStrParser<'r> {
    type Item = ValueField<'r>;

    fn next(&mut self) -> Option<Self::Item> {
        use std::borrow::Cow::*;

        let (name, value) = loop {
            if self.source.is_empty() {
                return None;
            }

            let (field_str, rest) = self.source.split_at_byte(b'&');
            self.source = rest;

            if !field_str.is_empty() {
                break field_str.split_at_byte(b'=');
            }
        };

        let name_val = match (name.url_decode_lossy(), value.url_decode_lossy()) {
            (Borrowed(name), Borrowed(val)) => (name, val),
            (Borrowed(name), Owned(v)) => (name, self.buffer.push_one(v)),
            (Owned(name), Borrowed(val)) => (self.buffer.push_one(name), val),
            (Owned(mut name), Owned(val)) => {
                let len = name.len();
                name.push_str(&val);
                self.buffer.push_split(name, len)
            }
        };

        Some(ValueField::from(name_val))
    }
}

#[cfg(test)]
mod raw_str_parse_tests {
    use crate::form::ValueField as Field;

    #[test]
    fn test_skips_empty() {
        let buffer = super::Buffer::new();
        let fields: Vec<_> = super::RawStrParser::new(&buffer, "a&b=c&&&c".into()).collect();
        assert_eq!(fields, &[Field::parse("a"), Field::parse("b=c"), Field::parse("c")]);
    }

    #[test]
    fn test_decodes() {
        let buffer = super::Buffer::new();
        let fields: Vec<_> = super::RawStrParser::new(&buffer, "a+b=c%20d&%26".into()).collect();
        assert_eq!(fields, &[Field::parse("a b=c d"), Field::parse("&")]);
    }
}

impl<'r, 'i> MultipartParser<'r, 'i> {
    /// Returns `None` when there are no further fields. Otherwise tries to
    /// parse the next multipart form field and returns the result.
    async fn next(&mut self) -> Option<Result<'r, Field<'r, 'i>>> {
        if self.done {
            return None;
        }

        let field = match self.source.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => return None,
            Err(e) => {
                self.done = true;
                return Some(Err(e.into()));
            }
        };

        // A field with a content-type is data; one without is "value".
        trace!(field_name=?field.name(), "multipart field");
        let content_type = field.content_type().and_then(|m| m.as_ref().parse().ok());
        let field = if let Some(content_type) = content_type {
            let (name, file_name) = match (field.name(), field.file_name()) {
                (None, None) => ("", None),
                (None, Some(file_name)) => ("", Some(self.buffer.push_one(file_name))),
                (Some(name), None) => (self.buffer.push_one(name), None),
                (Some(a), Some(b)) => {
                    let (field_name, file_name) = self.buffer.push_two(a, b);
                    (field_name, Some(file_name))
                }
            };

            Either::Right(DataField {
                content_type,
                request: self.request,
                name: NameView::new(name),
                file_name: file_name.map(FileName::new),
                data: Data::from(field),
            })
        } else {
            let (mut buf, len) = match field.name() {
                Some(s) => (s.to_string(), s.len()),
                None => (String::new(), 0)
            };

            match field.text().await {
                Ok(text) => buf.push_str(&text),
                Err(e) => return Some(Err(e.into())),
            };

            let name_val = self.buffer.push_split(buf, len);
            Either::Left(ValueField::from(name_val))
        };

        Some(Ok(field))
    }
}

impl Buffer {
    pub fn new() -> Self {
        Buffer {
            strings: UnsafeCell::new(vec![]),
            mutex: RawMutex::INIT,
        }
    }

    pub fn push_one<'a, S: Into<String>>(&'a self, string: S) -> &'a str {
        // SAFETY:
        //   * Aliasing: We retrieve a mutable reference to the last slot (via
        //     `push()`) and then return said reference as immutable; these
        //     occur in serial, so they don't alias. This method accesses a
        //     unique slot each call: the last slot, subsequently replaced by
        //     `push()` each next call. No other method accesses the internal
        //     buffer directly. Thus, the outstanding reference to the last slot
        //     is never accessed again mutably, preserving aliasing guarantees.
        //   * Liveness: The returned reference is to a `String`; we must ensure
        //     that the `String` is never dropped while `self` lives. This is
        //     guaranteed by returning a reference with the same lifetime as
        //     `self`, so `self` can't be dropped while the string is live, and
        //     by never removing elements from the internal `Vec` thus not
        //     dropping `String` itself: `push()` is the only mutating operation
        //     called on `Vec`, which preserves all previous elements; the
        //     stability of `String` itself means that the returned address
        //     remains valid even after internal realloc of `Vec`.
        //   * Thread-Safety: Parallel calls to `push_one` without exclusion
        //     would result in a race to `vec.push()`; `RawMutex` ensures that
        //     this doesn't occur.
        unsafe {
            self.mutex.lock();
            let vec: &mut Vec<String> = &mut *self.strings.get();
            vec.push(string.into());
            let last = vec.last().expect("push() => non-empty");
            self.mutex.unlock();
            last
        }
    }

    pub fn push_split(&self, string: String, len: usize) -> (&str, &str) {
        let buffered = self.push_one(string);
        let a = &buffered[..len];
        let b = &buffered[len..];
        (a, b)
    }

    pub fn push_two<'a>(&'a self, a: &str, b: &str) -> (&'a str, &'a str) {
        let mut buffer = String::new();
        buffer.push_str(a);
        buffer.push_str(b);

        self.push_split(buffer, a.len())
    }
}

unsafe impl Sync for Buffer {}
