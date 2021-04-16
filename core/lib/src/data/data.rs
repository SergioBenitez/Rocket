use crate::tokio::io::AsyncReadExt;
use crate::data::data_stream::DataStream;
use crate::data::{ByteUnit, StreamReader};

/// The number of bytes to read into the "peek" buffer.
pub const PEEK_BYTES: usize = 512;

/// Type representing the body data of a request.
///
/// This type is the only means by which the body of a request can be retrieved.
/// This type is not usually used directly. Instead, data guards (types that
/// implement [`FromData`](crate::data::FromData)) are created indirectly via
/// code generation by specifying the `data = "<var>"` route parameter as
/// follows:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # type DataGuard = rocket::data::Data;
/// #[post("/submit", data = "<var>")]
/// fn submit(var: DataGuard) { /* ... */ }
/// # fn main() { }
/// ```
///
/// Above, `DataGuard` can be any type that implements `FromData`. Note that
/// `Data` itself implements `FromData`.
///
/// # Reading Data
///
/// Data may be read from a `Data` object by calling either the
/// [`open()`](Data::open()) or [`peek()`](Data::peek()) methods.
///
/// The `open` method consumes the `Data` object and returns the raw data
/// stream. The `Data` object is consumed for safety reasons: consuming the
/// object ensures that holding a `Data` object means that all of the data is
/// available for reading.
///
/// The `peek` method returns a slice containing at most 512 bytes of buffered
/// body data. This enables partially or fully reading from a `Data` object
/// without consuming the `Data` object.
pub struct Data {
    buffer: Vec<u8>,
    is_complete: bool,
    stream: StreamReader,
}

impl Data {
    /// Create a `Data` from a recognized `stream`.
    pub(crate) fn from<S: Into<StreamReader>>(stream: S) -> Data {
        // TODO.async: This used to also set the read timeout to 5 seconds.
        // Such a short read timeout is likely no longer necessary, but some
        // kind of idle timeout should be implemented.

        let stream = stream.into();
        let buffer = Vec::with_capacity(PEEK_BYTES / 8);
        Data { buffer, stream, is_complete: false }
    }

    /// This creates a `data` object from a local data source `data`.
    #[inline]
    pub(crate) fn local(data: Vec<u8>) -> Data {
        Data {
            buffer: data,
            stream: StreamReader::empty(),
            is_complete: true,
        }
    }

    /// Returns the raw data stream, limited to `limit` bytes.
    ///
    /// The stream contains all of the data in the body of the request,
    /// including that in the `peek` buffer. The method consumes the `Data`
    /// instance. This ensures that a `Data` type _always_ represents _all_ of
    /// the data in a request.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::data::{Data, ToByteUnit};
    ///
    /// # const SIZE_LIMIT: u64 = 2 << 20; // 2MiB
    /// fn handler(data: Data) {
    ///     let stream = data.open(2.mebibytes());
    /// }
    /// ```
    pub fn open(self, limit: ByteUnit) -> DataStream {
        DataStream::new(self.buffer, self.stream, limit.into())
    }

    /// Retrieve at most `num` bytes from the `peek` buffer without consuming
    /// `self`.
    ///
    /// The peek buffer contains at most 512 bytes of the body of the request.
    /// The actual size of the returned buffer is the `max` of the request's
    /// body, `num` and `512`. The [`peek_complete`](#method.peek_complete)
    /// method can be used to determine if this buffer contains _all_ of the
    /// data in the body of the request.
    ///
    /// # Examples
    ///
    /// In a data guard:
    ///
    /// ```rust
    /// use rocket::request::{self, Request, FromRequest};
    /// use rocket::data::{self, Data, FromData};
    /// # struct MyType;
    /// # type MyError = String;
    ///
    /// #[rocket::async_trait]
    /// impl<'r> FromData<'r> for MyType {
    ///     type Error = MyError;
    ///
    ///     async fn from_data(
    ///         req: &'r Request<'_>,
    ///         mut data: Data
    ///     ) -> data::Outcome<Self, Self::Error> {
    ///         if data.peek(2).await != b"hi" {
    ///             return data::Outcome::Forward(data)
    ///         }
    ///
    ///         /* .. */
    ///         # unimplemented!()
    ///     }
    /// }
    /// ```
    ///
    /// In a fairing:
    ///
    /// ```
    /// use rocket::{Rocket, Request, Data, Response};
    /// use rocket::fairing::{Fairing, Info, Kind};
    /// # struct MyType;
    ///
    /// #[rocket::async_trait]
    /// impl Fairing for MyType {
    ///     fn info(&self) -> Info {
    ///         Info {
    ///             name: "Data Peeker",
    ///             kind: Kind::Request
    ///         }
    ///     }
    ///
    ///     async fn on_request(&self, req: &mut Request<'_>, data: &mut Data) {
    ///         if data.peek(2).await == b"hi" {
    ///             /* do something; body data starts with `"hi"` */
    ///         }
    ///
    ///         /* .. */
    ///         # unimplemented!()
    ///     }
    /// }
    /// ```
    pub async fn peek(&mut self, num: usize) -> &[u8] {
        let num = std::cmp::min(PEEK_BYTES, num);
        let mut len = self.buffer.len();
        if len >= num {
            return &self.buffer[..num];
        }

        while len < num {
            match self.stream.read_buf(&mut self.buffer).await {
                Ok(0) => { self.is_complete = true; break },
                Ok(n) => len += n,
                Err(e) => {
                    error!("Failed to read into peek buffer: {:?}.", e);
                    break;
                }
            }
        }

        &self.buffer[..std::cmp::min(len, num)]
    }

    /// Returns true if the `peek` buffer contains all of the data in the body
    /// of the request. Returns `false` if it does not or if it is not known if
    /// it does.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::data::Data;
    ///
    /// async fn handler(mut data: Data) {
    ///     if data.peek_complete() {
    ///         println!("All of the data: {:?}", data.peek(512).await);
    ///     }
    /// }
    /// ```
    #[inline(always)]
    pub fn peek_complete(&self) -> bool {
        self.is_complete
    }
}