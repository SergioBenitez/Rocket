//! Types and traits for request parsing and handling.

mod request;
mod from_param;
mod from_request;

#[cfg(test)]
mod tests;

pub use self::request::Request;
pub use self::from_request::{FromRequest, Outcome, FromWebSocket, WsOutcome};
pub use self::from_param::{FromParam, FromSegments};

#[doc(inline)]
pub use crate::response::flash::FlashMessage;

crate::export! {
    /// Store and immediately retrieve a value `$v` in `$request`'s local cache
    /// using a locally generated anonymous type to avoid type conflicts.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::request::local_cache;
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let request = c.get("/");
    ///
    /// // The first store into local cache for a given type wins.
    /// assert_eq!(request.local_cache(|| String::from("hello")), "hello");
    ///
    /// // The following returns the cached, previously stored value for the type.
    /// assert_eq!(request.local_cache(|| String::from("goodbye")), "hello");
    ///
    /// // This shows that we cannot cache different values of the same type; we
    /// // _must_ use a proxy type. To avoid the need to write these manually, use
    /// // `local_cache!`, which generates one of the fly.
    /// assert_eq!(local_cache!(request, String::from("hello")), "hello");
    /// assert_eq!(local_cache!(request, String::from("goodbye")), "goodbye");
    /// ```
    macro_rules! local_cache {
        ($request:expr, $v:expr $(,)?) => ({
            struct Local<T>(T);
            &$request.local_cache(move || Local($v)).0
        })
    }
}
