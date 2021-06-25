//! Re-exported hyper HTTP library types.
//!
//! All types that are re-exported from Hyper reside inside of this module.
//! These types will, with certainty, be removed with time, but they reside here
//! while necessary.

#[doc(hidden)] pub use hyper::{Body, Error, Request, Response};
#[doc(hidden)] pub use hyper::body::{Bytes, HttpBody, Sender as BodySender};
#[doc(hidden)] pub use hyper::rt::Executor;
#[doc(hidden)] pub use hyper::server::Server;
#[doc(hidden)] pub use hyper::service::{make_service_fn, service_fn, Service};
#[doc(hidden)] pub use hyper::upgrade;

#[doc(hidden)] pub use http::header::HeaderMap;
#[doc(hidden)] pub use http::header::HeaderName as HeaderName;
#[doc(hidden)] pub use http::header::HeaderValue as HeaderValue;
#[doc(hidden)] pub use http::method::Method;
#[doc(hidden)] pub use http::request::Parts as RequestParts;
#[doc(hidden)] pub use http::response::Builder as ResponseBuilder;
#[doc(hidden)] pub use http::status::StatusCode;
#[doc(hidden)] pub use http::uri::{Uri, Parts as UriParts};

/// Reexported http header types.
pub mod header {
    macro_rules! import_http_headers {
        ($($name:ident),*) => ($(
            pub use http::header::$name as $name;
        )*)
    }

    import_http_headers! {
        ACCEPT, ACCEPT_CHARSET, ACCEPT_ENCODING, ACCEPT_LANGUAGE, ACCEPT_RANGES,
        ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS,
        ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
        ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE,
        ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD, ALLOW,
        AUTHORIZATION, CACHE_CONTROL, CONNECTION, CONTENT_DISPOSITION,
        CONTENT_ENCODING, CONTENT_LANGUAGE, CONTENT_LENGTH, CONTENT_LOCATION,
        CONTENT_RANGE, CONTENT_SECURITY_POLICY,
        CONTENT_SECURITY_POLICY_REPORT_ONLY, CONTENT_TYPE, DATE, ETAG, EXPECT,
        EXPIRES, FORWARDED, FROM, HOST, IF_MATCH, IF_MODIFIED_SINCE,
        IF_NONE_MATCH, IF_RANGE, IF_UNMODIFIED_SINCE, LAST_MODIFIED, LINK,
        LOCATION, ORIGIN, PRAGMA, RANGE, REFERER, REFERRER_POLICY, REFRESH,
        STRICT_TRANSPORT_SECURITY, TE, TRANSFER_ENCODING, UPGRADE, USER_AGENT,
        VARY
    }
}
