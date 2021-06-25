use std::pin::Pin;
use std::task::{Context, Poll};
use std::path::Path;
use std::io::{self, Cursor};

use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt, ReadBuf, Take};
use futures::stream::Stream;
use futures::ready;
use tokio::sync::mpsc;

use crate::http::hyper;
use crate::ext::{PollExt, Chain};
use crate::data::{Capped, N};
use crate::websocket::message::WebSocketMessage;

/// Raw data stream of a request body.
///
/// This stream can only be obtained by calling
/// [`Data::open()`](crate::data::Data::open()) with a data limit. The stream
/// contains all of the data in the body of the request.
///
/// Reading from a `DataStream` is accomplished via the various methods on the
/// structure. In general, methods exists in two variants: those that _check_
/// whether the entire stream was read and those that don't. The former either
/// directly or indirectly (via [`Capped`]) return an [`N`] which allows
/// checking if the stream was read to completion while the latter do not.
///
/// | Read Into | Method                               | Notes                            |
/// |-----------|--------------------------------------|----------------------------------|
/// | `String`  | [`DataStream::into_string()`]        | Completeness checked. Preferred. |
/// | `String`  | [`AsyncReadExt::read_to_string()`]   | Unchecked w/existing `String`.   |
/// | `Vec<u8>` | [`DataStream::into_bytes()`]         | Checked. Preferred.              |
/// | `Vec<u8>` | [`DataStream::stream_to(&mut vec)`]  | Checked w/existing `Vec`.        |
/// | `Vec<u8>` | [`DataStream::stream_precise_to()`]  | Unchecked w/existing `Vec`.      |
/// | `File`    | [`DataStream::into_file()`]          | Checked. Preferred.              |
/// | `File`    | [`DataStream::stream_to(&mut file)`] | Checked w/ existing `File`.      |
/// | `File`    | [`DataStream::stream_precise_to()`]  | Unchecked w/ existing `File`.    |
/// | `T`       | [`DataStream::stream_to()`]          | Checked. Any `T: AsyncWrite`.    |
/// | `T`       | [`DataStream::stream_precise_to()`]  | Unchecked. Any `T: AsyncWrite`.  |
///
/// [`DataStream::stream_to(&mut vec)`]: DataStream::stream_to()
/// [`DataStream::stream_to(&mut file)`]: DataStream::stream_to()
pub struct DataStream<'r> {
    pub(crate) chain: Take<Chain<Cursor<Vec<u8>>, StreamReader<'r>>>,
}

/// An adapter: turns a `T: Stream` (in `StreamKind`) into a `tokio::AsyncRead`.
pub struct StreamReader<'r> {
    state: State,
    inner: StreamKind<'r>,
}

/// The current state of `StreamReader` `AsyncRead` adapter.
enum State {
    Pending,
    Partial(Cursor<hyper::Bytes>),
    Done,
}

/// The kinds of streams we accept as `Data`.
enum StreamKind<'r> {
    Empty,
    Body(&'r mut hyper::Body),
    Multipart(multer::Field<'r>),
    WebSocket(mpsc::Receiver<hyper::Bytes>),
}

impl<'r> DataStream<'r> {
    pub(crate) fn new(buf: Vec<u8>, stream: StreamReader<'r>, limit: u64) -> Self {
        let chain = Chain::new(Cursor::new(buf), stream).take(limit);
        Self { chain }
    }

    /// Whether a previous read exhausted the set limit _and then some_.
    async fn limit_exceeded(&mut self) -> io::Result<bool> {
        #[cold]
        async fn _limit_exceeded(stream: &mut DataStream<'_>) -> io::Result<bool> {
            stream.chain.set_limit(1);
            let mut buf = [0u8; 1];
            Ok(stream.read(&mut buf).await? != 0)
        }

        Ok(self.chain.limit() == 0 && _limit_exceeded(self).await?)
    }

    /// Number of bytes a full read from `self` will _definitely_ read.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::data::{Data, ToByteUnit};
    ///
    /// async fn f(data: Data<'_>) {
    ///     let definitely_have_n_bytes = data.open(1.kibibytes()).hint();
    /// }
    /// ```
    pub fn hint(&self) -> usize {
        let buf_len = self.chain.get_ref().get_ref().0.get_ref().len();
        std::cmp::min(buf_len, self.chain.limit() as usize)
    }

    /// A helper method to write the body of the request to any `AsyncWrite`
    /// type. Returns an [`N`] which indicates how many bytes were written and
    /// whether the entire stream was read. An additional read from `self` may
    /// be required to check if all of the sream has been read. If that
    /// information is not needed, use [`DataStream::stream_precise_to()`].
    ///
    /// This method is identical to `tokio::io::copy(&mut self, &mut writer)`
    /// except in that it returns an `N` to check for completeness.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::io;
    /// use rocket::data::{Data, ToByteUnit};
    ///
    /// async fn data_guard(mut data: Data<'_>) -> io::Result<String> {
    ///     // write all of the data to stdout
    ///     let written = data.open(512.kibibytes())
    ///         .stream_to(tokio::io::stdout()).await?;
    ///
    ///     Ok(format!("Wrote {} bytes.", written))
    /// }
    /// ```
    #[inline(always)]
    pub async fn stream_to<W>(mut self, mut writer: W) -> io::Result<N>
        where W: AsyncWrite + Unpin
    {
        let written = tokio::io::copy(&mut self, &mut writer).await?;
        Ok(N { written, complete: !self.limit_exceeded().await? })
    }

    /// Like [`DataStream::stream_to()`] except that no end-of-stream check is
    /// conducted and thus read/write completeness is unknown.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::io;
    /// use rocket::data::{Data, ToByteUnit};
    ///
    /// async fn data_guard(mut data: Data<'_>) -> io::Result<String> {
    ///     // write all of the data to stdout
    ///     let written = data.open(512.kibibytes())
    ///         .stream_precise_to(tokio::io::stdout()).await?;
    ///
    ///     Ok(format!("Wrote {} bytes.", written))
    /// }
    /// ```
    #[inline(always)]
    pub async fn stream_precise_to<W>(mut self, mut writer: W) -> io::Result<u64>
        where W: AsyncWrite + Unpin
    {
        tokio::io::copy(&mut self, &mut writer).await
    }

    /// A helper method to write the body of the request to a `Vec<u8>`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::io;
    /// use rocket::data::{Data, ToByteUnit};
    ///
    /// async fn data_guard(data: Data<'_>) -> io::Result<Vec<u8>> {
    ///     let bytes = data.open(4.kibibytes()).into_bytes().await?;
    ///     if !bytes.is_complete() {
    ///         println!("there are bytes remaining in the stream");
    ///     }
    ///
    ///     Ok(bytes.into_inner())
    /// }
    /// ```
    pub async fn into_bytes(self) -> io::Result<Capped<Vec<u8>>> {
        let mut vec = Vec::with_capacity(self.hint());
        let n = self.stream_to(&mut vec).await?;
        Ok(Capped { value: vec, n })
    }

    /// A helper method to write the body of the request to a `String`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::io;
    /// use rocket::data::{Data, ToByteUnit};
    ///
    /// async fn data_guard(data: Data<'_>) -> io::Result<String> {
    ///     let string = data.open(10.bytes()).into_string().await?;
    ///     if !string.is_complete() {
    ///         println!("there are bytes remaining in the stream");
    ///     }
    ///
    ///     Ok(string.into_inner())
    /// }
    /// ```
    pub async fn into_string(mut self) -> io::Result<Capped<String>> {
        let mut string = String::with_capacity(self.hint());
        let written = self.read_to_string(&mut string).await?;
        let n = N { written: written as u64, complete: !self.limit_exceeded().await? };
        Ok(Capped { value: string, n })
    }

    /// A helper method to write the body of the request to a file at the path
    /// determined by `path`. If a file at the path already exists, it is
    /// overwritten. The opened file is returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::io;
    /// use rocket::data::{Data, ToByteUnit};
    ///
    /// async fn data_guard(mut data: Data<'_>) -> io::Result<String> {
    ///     let file = data.open(1.megabytes()).into_file("/static/file").await?;
    ///     if !file.is_complete() {
    ///         println!("there are bytes remaining in the stream");
    ///     }
    ///
    ///     Ok(format!("Wrote {} bytes to /static/file", file.n))
    /// }
    /// ```
    pub async fn into_file<P: AsRef<Path>>(self, path: P) -> io::Result<Capped<File>> {
        let mut file = File::create(path).await?;
        let n = self.stream_to(&mut tokio::io::BufWriter::new(&mut file)).await?;
        Ok(Capped { value: file, n })
    }
}

// TODO.async: Consider implementing `AsyncBufRead`.

impl StreamReader<'_> {
    pub fn empty() -> Self {
        Self { inner: StreamKind::Empty, state: State::Done }
    }
}

impl<'r> From<&'r mut hyper::Body> for StreamReader<'r> {
    fn from(body: &'r mut hyper::Body) -> Self {
        Self { inner: StreamKind::Body(body), state: State::Pending }
    }
}

impl<'r> From<multer::Field<'r>> for StreamReader<'r> {
    fn from(field: multer::Field<'r>) -> Self {
        Self { inner: StreamKind::Multipart(field), state: State::Pending }
    }
}

impl<'r> From<WebSocketMessage> for StreamReader<'r> {
    fn from(message: WebSocketMessage) -> Self {
        Self { inner: StreamKind::WebSocket(message.inner()), state: State::Pending }
    }
}

impl AsyncRead for DataStream<'_> {
    #[inline(always)]
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.chain).poll_read(cx, buf)
    }
}

impl Stream for StreamKind<'_> {
    type Item = io::Result<hyper::Bytes>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            StreamKind::Body(body) => Pin::new(body).poll_next(cx)
                .map_err_ext(|e| io::Error::new(io::ErrorKind::Other, e)),
            StreamKind::Multipart(mp) => Pin::new(mp).poll_next(cx)
                .map_err_ext(|e| io::Error::new(io::ErrorKind::Other, e)),
            StreamKind::Empty => Poll::Ready(None),
            StreamKind::WebSocket(rx) => Pin::new(rx).poll_recv(cx)
                .map(|b| b.map(|b| Ok(b))),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            StreamKind::Body(body) => body.size_hint(),
            StreamKind::Multipart(mp) => mp.size_hint(),
            StreamKind::Empty => (0, Some(0)),
            StreamKind::WebSocket(_rx) => (0, Some(0)),
        }
    }
}

impl AsyncRead for StreamReader<'_> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            self.state = match self.state {
                State::Pending => {
                    match ready!(Pin::new(&mut self.inner).poll_next(cx)) {
                        Some(Err(e)) => return Poll::Ready(Err(e)),
                        Some(Ok(bytes)) => State::Partial(Cursor::new(bytes)),
                        None => State::Done,
                    }
                },
                State::Partial(ref mut cursor) => {
                    let rem = buf.remaining();
                    match ready!(Pin::new(cursor).poll_read(cx, buf)) {
                        Ok(()) if rem == buf.remaining() => State::Pending,
                        result => return Poll::Ready(result),
                    }
                }
                State::Done => return Poll::Ready(Ok(())),
            }
        }
    }
}
