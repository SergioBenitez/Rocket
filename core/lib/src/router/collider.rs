use crate::catcher::Catcher;
use crate::route::{Route, Color};

use crate::http::{MediaType, Status};
use crate::request::Request;

pub trait Collide<T = Self> {
    fn collides_with(&self, other: &T) -> bool;
}

impl<'a, 'b, T: Collide> Collide<&T> for &T {
    fn collides_with(&self, other: &&T) -> bool {
        T::collides_with(*self, *other)
    }
}

impl Collide for MediaType {
    fn collides_with(&self, other: &Self) -> bool {
        let collide = |a, b| a == "*" || b == "*" || a == b;
        collide(self.top(), other.top()) && collide(self.sub(), other.sub())
    }
}

fn paths_collide(route: &Route, other: &Route) -> bool {
    let a_segments = &route.uri.metadata.path_segs;
    let b_segments = &other.uri.metadata.path_segs;
    for (seg_a, seg_b) in a_segments.iter().zip(b_segments.iter()) {
        if seg_a.trailing || seg_b.trailing {
            return true;
        }

        if seg_a.dynamic || seg_b.dynamic {
            continue;
        }

        if seg_a.value != seg_b.value {
            return false;
        }
    }

    a_segments.get(b_segments.len()).map_or(false, |s| s.trailing)
        || b_segments.get(a_segments.len()).map_or(false, |s| s.trailing)
        || a_segments.len() == b_segments.len()
}

fn formats_collide(route: &Route, other: &Route) -> bool {
    // When matching against the `Accept` header, the client can always provide
    // a media type that will cause a collision through non-specificity, i.e,
    // `*/*` matches everything.
    if !route.method.supports_payload() {
        return true;
    }

    // When matching against the `Content-Type` header, we'll only consider
    // requests as having a `Content-Type` if they're fully specified. If a
    // route doesn't have a `format`, it accepts all `Content-Type`s. If a
    // request doesn't have a format, it only matches routes without a format.
    match (route.format.as_ref(), other.format.as_ref()) {
        (Some(a), Some(b)) => a.collides_with(b),
        _ => true
    }
}

impl Collide for Route {
    /// Determines if two routes can match against some request. That is, if two
    /// routes `collide`, there exists a request that can match against both
    /// routes.
    ///
    /// This implementation is used at initialization to check if two user
    /// routes collide before launching. Format collisions works like this:
    ///
    ///   * If route specifies a format, it only gets requests for that format.
    ///   * If route doesn't specify a format, it gets requests for any format.
    ///
    /// Because query parsing is lenient, and dynamic query parameters can be
    /// missing, queries do not impact whether two routes collide.
    fn collides_with(&self, other: &Route) -> bool {
        self.method == other.method
            && self.rank == other.rank
            && paths_collide(self, other)
            && formats_collide(self, other)
    }
}

impl Route {
    /// Determines if this route matches against the given request.
    ///
    /// This means that:
    ///
    ///   * The route's method matches that of the incoming request.
    ///   * The route's format (if any) matches that of the incoming request.
    ///     - If route specifies format, it only gets requests for that format.
    ///     - If route doesn't specify format, it gets requests for any format.
    ///   * All static components in the route's path match the corresponding
    ///     components in the same position in the incoming request.
    ///   * All static components in the route's query string are also in the
    ///     request query string, though in any position. If there is no query
    ///     in the route, requests with/without queries match.
    pub(crate) fn matches(&self, req: &Request<'_>) -> bool {
        self.method == req.method()
            && paths_match(self, req)
            && queries_match(self, req)
            && formats_match(self, req)
    }
}

fn paths_match(route: &Route, req: &Request<'_>) -> bool {
    let route_segments = &route.uri.metadata.path_segs;
    let req_segments = req.uri().path_segments();

    if route.uri.metadata.trailing_path {
        // The last route segment can be trailing, which is allowed to be empty.
        // So we can have one more segment in `route` than in `req` and match.
        // ok if: req_segments.len() >= route_segments.len() - 1
        if req_segments.len() + 1 < route_segments.len() {
            return false;
        }
    } else if route_segments.len() != req_segments.len() {
        return false;
    }

    if route.uri.metadata.path_color == Color::Wild {
        return true;
    }

    for (route_seg, req_seg) in route_segments.iter().zip(req_segments) {
        if route_seg.trailing {
            return true;
        }

        if !(route_seg.dynamic || route_seg.value == req_seg) {
            return false;
        }
    }

    true
}

fn queries_match(route: &Route, req: &Request<'_>) -> bool {
    if matches!(route.uri.metadata.query_color, None | Some(Color::Wild)) {
        return true;
    }

    let route_query_fields = route.uri.metadata.static_query_fields.iter()
        .map(|(k, v)| (k.as_str(), v.as_str()));

    for route_seg in route_query_fields {
        if !req.uri().query_segments().any(|req_seg| req_seg == route_seg) {
            trace!("request {} missing static query {:?}", req, route_seg);
            return false;
        }
    }

    true
}

fn formats_match(route: &Route, request: &Request<'_>) -> bool {
    if !route.method.supports_payload() {
        route.format.as_ref()
            .and_then(|a| request.format().map(|b| (a, b)))
            .map(|(a, b)| a.collides_with(b))
            .unwrap_or(true)
    } else {
        match route.format.as_ref() {
            Some(a) => match request.format() {
                Some(b) if b.specificity() == 2 => a.collides_with(b),
                _ => false
            }
            None => true
        }
    }
}


impl Collide for Catcher {
    /// Determines if two catchers are in conflict: there exists a request for
    /// which there exist no rule to determine _which_ of the two catchers to
    /// use. This means that the catchers:
    ///
    ///  * Have the same base.
    ///  * Have the same status code or are both defaults.
    fn collides_with(&self, other: &Self) -> bool {
        self.code == other.code
            && self.base.path_segments().eq(other.base.path_segments())
    }
}

impl Catcher {
    /// Determines if this catcher is responsible for handling the error with
    /// `status` that occurred during request `req`. A catcher matches if:
    ///
    ///  * It is a default catcher _or_ has a code of `status`.
    ///  * Its base is a prefix of the normalized/decoded `req.path()`.
    pub(crate) fn matches(&self, status: Status, req: &Request<'_>) -> bool {
        self.code.map_or(true, |code| code == status.code)
            && self.base.path_segments().prefix_of(req.uri().path_segments())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::route::{Route, dummy_handler};
    use crate::local::blocking::Client;
    use crate::http::{Method, Method::*, MediaType, ContentType, Accept};
    use crate::http::uri::Origin;

    type SimpleRoute = (Method, &'static str);

    fn m_collide(a: SimpleRoute, b: SimpleRoute) -> bool {
        let route_a = Route::new(a.0, a.1, dummy_handler);
        route_a.collides_with(&Route::new(b.0, b.1, dummy_handler))
    }

    fn unranked_collide(a: &'static str, b: &'static str) -> bool {
        let route_a = Route::ranked(0, Get, a, dummy_handler);
        let route_b = Route::ranked(0, Get, b, dummy_handler);
        eprintln!("Checking {} against {}.", route_a, route_b);
        route_a.collides_with(&route_b)
    }

    fn s_s_collide(a: &'static str, b: &'static str) -> bool {
        let a = Route::new(Get, a, dummy_handler);
        let b = Route::new(Get, b, dummy_handler);
        paths_collide(&a, &b)
    }

    #[test]
    fn simple_collisions() {
        assert!(unranked_collide("/a", "/a"));
        assert!(unranked_collide("/hello", "/hello"));
        assert!(unranked_collide("/hello", "/hello/"));
        assert!(unranked_collide("/hello/there/how/ar", "/hello/there/how/ar"));
        assert!(unranked_collide("/hello/there", "/hello/there/"));
    }

    #[test]
    fn simple_param_collisions() {
        assert!(unranked_collide("/<a>", "/<b>"));
        assert!(unranked_collide("/<a>", "/b"));
        assert!(unranked_collide("/hello/<name>", "/hello/<person>"));
        assert!(unranked_collide("/hello/<name>/hi", "/hello/<person>/hi"));
        assert!(unranked_collide("/hello/<name>/hi/there", "/hello/<person>/hi/there"));
        assert!(unranked_collide("/<name>/hi/there", "/<person>/hi/there"));
        assert!(unranked_collide("/<name>/hi/there", "/dude/<name>/there"));
        assert!(unranked_collide("/<name>/<a>/<b>", "/<a>/<b>/<c>"));
        assert!(unranked_collide("/<name>/<a>/<b>/", "/<a>/<b>/<c>/"));
        assert!(unranked_collide("/<a..>", "/hi"));
        assert!(unranked_collide("/<a..>", "/hi/hey"));
        assert!(unranked_collide("/<a..>", "/hi/hey/hayo"));
        assert!(unranked_collide("/a/<a..>", "/a/hi/hey/hayo"));
        assert!(unranked_collide("/a/<b>/<a..>", "/a/hi/hey/hayo"));
        assert!(unranked_collide("/a/<b>/<c>/<a..>", "/a/hi/hey/hayo"));
        assert!(unranked_collide("/<b>/<c>/<a..>", "/a/hi/hey/hayo"));
        assert!(unranked_collide("/<b>/<c>/hey/hayo", "/a/hi/hey/hayo"));
        assert!(unranked_collide("/<a..>", "/foo"));
    }

    #[test]
    fn medium_param_collisions() {
        assert!(unranked_collide("/<a>", "/b"));
        assert!(unranked_collide("/hello/<name>", "/hello/bob"));
        assert!(unranked_collide("/<name>", "//bob"));
    }

    #[test]
    fn hard_param_collisions() {
        assert!(unranked_collide("/<a..>", "///a///"));
        assert!(unranked_collide("/<a..>", "//a/bcjdklfj//<c>"));
        assert!(unranked_collide("/a/<a..>", "//a/bcjdklfj//<c>"));
        assert!(unranked_collide("/a/<b>/<c..>", "//a/bcjdklfj//<c>"));
        assert!(unranked_collide("/<a..>", "/"));
        assert!(unranked_collide("/", "/<_..>"));
        assert!(unranked_collide("/a/b/<a..>", "/a/<b..>"));
        assert!(unranked_collide("/a/b/<a..>", "/a/<b>/<b..>"));
        assert!(unranked_collide("/hi/<a..>", "/hi"));
        assert!(unranked_collide("/hi/<a..>", "/hi/"));
        assert!(unranked_collide("/<a..>", "//////"));
    }

    #[test]
    fn query_collisions() {
        assert!(unranked_collide("/?<a>", "/?<a>"));
        assert!(unranked_collide("/a/?<a>", "/a/?<a>"));
        assert!(unranked_collide("/a?<a>", "/a?<a>"));
        assert!(unranked_collide("/<r>?<a>", "/<r>?<a>"));
        assert!(unranked_collide("/a/b/c?<a>", "/a/b/c?<a>"));
        assert!(unranked_collide("/<a>/b/c?<d>", "/a/b/<c>?<d>"));
        assert!(unranked_collide("/?<a>", "/"));
        assert!(unranked_collide("/a?<a>", "/a"));
        assert!(unranked_collide("/a?<a>", "/a"));
        assert!(unranked_collide("/a/b?<a>", "/a/b"));
        assert!(unranked_collide("/a/b", "/a/b?<c>"));
    }

    #[test]
    fn non_collisions() {
        assert!(!unranked_collide("/<a>", "/"));
        assert!(!unranked_collide("/a", "/b"));
        assert!(!unranked_collide("/a/b", "/a"));
        assert!(!unranked_collide("/a/b", "/a/c"));
        assert!(!unranked_collide("/a/hello", "/a/c"));
        assert!(!unranked_collide("/hello", "/a/c"));
        assert!(!unranked_collide("/hello/there", "/hello/there/guy"));
        assert!(!unranked_collide("/a/<b>", "/b/<b>"));
        assert!(!unranked_collide("/t", "/test"));
        assert!(!unranked_collide("/a", "/aa"));
        assert!(!unranked_collide("/a", "/aaa"));
        assert!(!unranked_collide("/", "/a"));
    }

    #[test]
    fn query_non_collisions() {
        assert!(!unranked_collide("/a?<b>", "/b"));
        assert!(!unranked_collide("/a/b", "/a?<b>"));
        assert!(!unranked_collide("/a/b/c?<d>", "/a/b/c/d"));
        assert!(!unranked_collide("/a/hello", "/a/?<hello>"));
        assert!(!unranked_collide("/?<a>", "/hi"));
    }

    #[test]
    fn method_dependent_non_collisions() {
        assert!(!m_collide((Get, "/"), (Post, "/")));
        assert!(!m_collide((Post, "/"), (Put, "/")));
        assert!(!m_collide((Put, "/a"), (Put, "/")));
        assert!(!m_collide((Post, "/a"), (Put, "/")));
        assert!(!m_collide((Get, "/a"), (Put, "/")));
        assert!(!m_collide((Get, "/hello"), (Put, "/hello")));
        assert!(!m_collide((Get, "/<foo..>"), (Post, "/")));
    }

    #[test]
    fn query_dependent_non_collisions() {
        assert!(!m_collide((Get, "/"), (Get, "/?a")));
        assert!(!m_collide((Get, "/"), (Get, "/?<a>")));
        assert!(!m_collide((Get, "/a/<b>"), (Get, "/a/<b>?d")));
    }

    #[test]
    fn test_str_non_collisions() {
        assert!(!s_s_collide("/a", "/b"));
        assert!(!s_s_collide("/a/b", "/a"));
        assert!(!s_s_collide("/a/b", "/a/c"));
        assert!(!s_s_collide("/a/hello", "/a/c"));
        assert!(!s_s_collide("/hello", "/a/c"));
        assert!(!s_s_collide("/hello/there", "/hello/there/guy"));
        assert!(!s_s_collide("/a/<b>", "/b/<b>"));
        assert!(!s_s_collide("/a", "/b"));
        assert!(!s_s_collide("/a/b", "/a"));
        assert!(!s_s_collide("/a/b", "/a/c"));
        assert!(!s_s_collide("/a/hello", "/a/c"));
        assert!(!s_s_collide("/hello", "/a/c"));
        assert!(!s_s_collide("/hello/there", "/hello/there/guy"));
        assert!(!s_s_collide("/a/<b>", "/b/<b>"));
        assert!(!s_s_collide("/a", "/b"));
        assert!(!s_s_collide("/a/b", "/a"));
        assert!(!s_s_collide("/a/b", "/a/c"));
        assert!(!s_s_collide("/a/hello", "/a/c"));
        assert!(!s_s_collide("/hello", "/a/c"));
        assert!(!s_s_collide("/hello/there", "/hello/there/guy"));
        assert!(!s_s_collide("/a/<b>", "/b/<b>"));
        assert!(!s_s_collide("/t", "/test"));
        assert!(!s_s_collide("/a", "/aa"));
        assert!(!s_s_collide("/a", "/aaa"));
        assert!(!s_s_collide("/", "/a"));

        assert!(s_s_collide("/a/hi/<a..>", "/a/hi/"));
        assert!(s_s_collide("/hi/<a..>", "/hi/"));
        assert!(s_s_collide("/<a..>", "/"));
    }

    fn mt_mt_collide(mt1: &str, mt2: &str) -> bool {
        let mt_a = MediaType::from_str(mt1).expect(mt1);
        let mt_b = MediaType::from_str(mt2).expect(mt2);
        mt_a.collides_with(&mt_b)
    }

    #[test]
    fn test_content_type_colliions() {
        assert!(mt_mt_collide("application/json", "application/json"));
        assert!(mt_mt_collide("*/json", "application/json"));
        assert!(mt_mt_collide("*/*", "application/json"));
        assert!(mt_mt_collide("application/*", "application/json"));
        assert!(mt_mt_collide("application/*", "*/json"));
        assert!(mt_mt_collide("something/random", "something/random"));

        assert!(!mt_mt_collide("text/*", "application/*"));
        assert!(!mt_mt_collide("*/text", "*/json"));
        assert!(!mt_mt_collide("*/text", "application/test"));
        assert!(!mt_mt_collide("something/random", "something_else/random"));
        assert!(!mt_mt_collide("something/random", "*/else"));
        assert!(!mt_mt_collide("*/random", "*/else"));
        assert!(!mt_mt_collide("something/*", "random/else"));
    }

    fn r_mt_mt_collide<S1, S2>(m: Method, mt1: S1, mt2: S2) -> bool
        where S1: Into<Option<&'static str>>, S2: Into<Option<&'static str>>
    {
        let mut route_a = Route::new(m, "/", dummy_handler);
        if let Some(mt_str) = mt1.into() {
            route_a.format = Some(mt_str.parse::<MediaType>().unwrap());
        }

        let mut route_b = Route::new(m, "/", dummy_handler);
        if let Some(mt_str) = mt2.into() {
            route_b.format = Some(mt_str.parse::<MediaType>().unwrap());
        }

        route_a.collides_with(&route_b)
    }

    #[test]
    fn test_route_content_type_colliions() {
        // non-payload bearing routes always collide
        assert!(r_mt_mt_collide(Get, "application/json", "application/json"));
        assert!(r_mt_mt_collide(Get, "*/json", "application/json"));
        assert!(r_mt_mt_collide(Get, "*/json", "application/*"));
        assert!(r_mt_mt_collide(Get, "text/html", "text/*"));
        assert!(r_mt_mt_collide(Get, "any/thing", "*/*"));

        assert!(r_mt_mt_collide(Get, None, "text/*"));
        assert!(r_mt_mt_collide(Get, None, "text/html"));
        assert!(r_mt_mt_collide(Get, None, "*/*"));
        assert!(r_mt_mt_collide(Get, "text/html", None));
        assert!(r_mt_mt_collide(Get, "*/*", None));
        assert!(r_mt_mt_collide(Get, "application/json", None));

        assert!(r_mt_mt_collide(Get, "application/*", "text/*"));
        assert!(r_mt_mt_collide(Get, "application/json", "text/*"));
        assert!(r_mt_mt_collide(Get, "application/json", "text/html"));
        assert!(r_mt_mt_collide(Get, "text/html", "text/html"));

        // payload bearing routes collide if the media types collide
        assert!(r_mt_mt_collide(Post, "application/json", "application/json"));
        assert!(r_mt_mt_collide(Post, "*/json", "application/json"));
        assert!(r_mt_mt_collide(Post, "*/json", "application/*"));
        assert!(r_mt_mt_collide(Post, "text/html", "text/*"));
        assert!(r_mt_mt_collide(Post, "any/thing", "*/*"));

        assert!(r_mt_mt_collide(Post, None, "text/*"));
        assert!(r_mt_mt_collide(Post, None, "text/html"));
        assert!(r_mt_mt_collide(Post, None, "*/*"));
        assert!(r_mt_mt_collide(Post, "text/html", None));
        assert!(r_mt_mt_collide(Post, "*/*", None));
        assert!(r_mt_mt_collide(Post, "application/json", None));

        assert!(!r_mt_mt_collide(Post, "text/html", "application/*"));
        assert!(!r_mt_mt_collide(Post, "application/html", "text/*"));
        assert!(!r_mt_mt_collide(Post, "*/json", "text/html"));
        assert!(!r_mt_mt_collide(Post, "text/html", "text/css"));
        assert!(!r_mt_mt_collide(Post, "other/html", "text/html"));
    }

    fn req_route_mt_collide<S1, S2>(m: Method, mt1: S1, mt2: S2) -> bool
        where S1: Into<Option<&'static str>>, S2: Into<Option<&'static str>>
    {
        let client = Client::debug_with(vec![]).expect("client");
        let mut req = client.req(m, "/");
        if let Some(mt_str) = mt1.into() {
            if m.supports_payload() {
                req.replace_header(mt_str.parse::<ContentType>().unwrap());
            } else {
                req.replace_header(mt_str.parse::<Accept>().unwrap());
            }
        }

        let mut route = Route::new(m, "/", dummy_handler);
        if let Some(mt_str) = mt2.into() {
            route.format = Some(mt_str.parse::<MediaType>().unwrap());
        }

        route.matches(&req)
    }

    #[test]
    fn test_req_route_mt_collisions() {
        assert!(req_route_mt_collide(Post, "application/json", "application/json"));
        assert!(req_route_mt_collide(Post, "application/json", "application/*"));
        assert!(req_route_mt_collide(Post, "application/json", "*/json"));
        assert!(req_route_mt_collide(Post, "text/html", "*/*"));

        assert!(req_route_mt_collide(Get, "application/json", "application/json"));
        assert!(req_route_mt_collide(Get, "text/html", "text/html"));
        assert!(req_route_mt_collide(Get, "text/html", "*/*"));
        assert!(req_route_mt_collide(Get, None, "*/*"));
        assert!(req_route_mt_collide(Get, None, "text/*"));
        assert!(req_route_mt_collide(Get, None, "text/html"));
        assert!(req_route_mt_collide(Get, None, "application/json"));

        assert!(req_route_mt_collide(Post, "text/html", None));
        assert!(req_route_mt_collide(Post, "application/json", None));
        assert!(req_route_mt_collide(Post, "x-custom/anything", None));
        assert!(req_route_mt_collide(Post, None, None));

        assert!(req_route_mt_collide(Get, "text/html", None));
        assert!(req_route_mt_collide(Get, "application/json", None));
        assert!(req_route_mt_collide(Get, "x-custom/anything", None));
        assert!(req_route_mt_collide(Get, None, None));
        assert!(req_route_mt_collide(Get, None, "text/html"));
        assert!(req_route_mt_collide(Get, None, "application/json"));

        assert!(req_route_mt_collide(Get, "text/html, text/plain", "text/html"));
        assert!(req_route_mt_collide(Get, "text/html; q=0.5, text/xml", "text/xml"));

        assert!(!req_route_mt_collide(Post, None, "text/html"));
        assert!(!req_route_mt_collide(Post, None, "text/*"));
        assert!(!req_route_mt_collide(Post, None, "*/text"));
        assert!(!req_route_mt_collide(Post, None, "*/*"));
        assert!(!req_route_mt_collide(Post, None, "text/html"));
        assert!(!req_route_mt_collide(Post, None, "application/json"));

        assert!(!req_route_mt_collide(Post, "application/json", "text/html"));
        assert!(!req_route_mt_collide(Post, "application/json", "text/*"));
        assert!(!req_route_mt_collide(Post, "application/json", "*/xml"));
        assert!(!req_route_mt_collide(Get, "application/json", "text/html"));
        assert!(!req_route_mt_collide(Get, "application/json", "text/*"));
        assert!(!req_route_mt_collide(Get, "application/json", "*/xml"));

        assert!(!req_route_mt_collide(Post, None, "text/html"));
        assert!(!req_route_mt_collide(Post, None, "application/json"));
    }

    fn req_route_path_match(a: &'static str, b: &'static str) -> bool {
        let client = Client::debug_with(vec![]).expect("client");
        let req = client.get(Origin::parse(a).expect("valid URI"));
        let route = Route::ranked(0, Get, b, dummy_handler);
        route.matches(&req)
    }

    #[test]
    fn test_req_route_query_collisions() {
        assert!(req_route_path_match("/a/b?a=b", "/a/b?<c>"));
        assert!(req_route_path_match("/a/b?a=b", "/<a>/b?<c>"));
        assert!(req_route_path_match("/a/b?a=b", "/<a>/<b>?<c>"));
        assert!(req_route_path_match("/a/b?a=b", "/a/<b>?<c>"));
        assert!(req_route_path_match("/?b=c", "/?<b>"));

        assert!(req_route_path_match("/a/b?a=b", "/a/b"));
        assert!(req_route_path_match("/a/b", "/a/b"));
        assert!(req_route_path_match("/a/b/c/d?", "/a/b/c/d"));
        assert!(req_route_path_match("/a/b/c/d?v=1&v=2", "/a/b/c/d"));

        assert!(req_route_path_match("/a/b", "/a/b?<c>"));
        assert!(req_route_path_match("/a/b", "/a/b?<c..>"));
        assert!(req_route_path_match("/a/b?c", "/a/b?c"));
        assert!(req_route_path_match("/a/b?c", "/a/b?<c>"));
        assert!(req_route_path_match("/a/b?c=foo&d=z", "/a/b?<c>"));
        assert!(req_route_path_match("/a/b?c=foo&d=z", "/a/b?<c..>"));

        assert!(req_route_path_match("/a/b?c=foo&d=z", "/a/b?c=foo&<c..>"));
        assert!(req_route_path_match("/a/b?c=foo&d=z", "/a/b?d=z&<c..>"));

        assert!(!req_route_path_match("/a/b/c", "/a/b?<c>"));
        assert!(!req_route_path_match("/a?b=c", "/a/b?<c>"));
        assert!(!req_route_path_match("/?b=c", "/a/b?<c>"));
        assert!(!req_route_path_match("/?b=c", "/a?<c>"));

        assert!(!req_route_path_match("/a/b?c=foo&d=z", "/a/b?a=b&<c..>"));
        assert!(!req_route_path_match("/a/b?c=foo&d=z", "/a/b?d=b&<c..>"));
        assert!(!req_route_path_match("/a/b", "/a/b?c"));
        assert!(!req_route_path_match("/a/b", "/a/b?foo"));
        assert!(!req_route_path_match("/a/b", "/a/b?foo&<rest..>"));
        assert!(!req_route_path_match("/a/b", "/a/b?<a>&b&<rest..>"));
    }


    fn catchers_collide<A, B>(a: A, ap: &str, b: B, bp: &str) -> bool
        where A: Into<Option<u16>>, B: Into<Option<u16>>
    {
        use crate::catcher::dummy_handler as handler;

        let a = Catcher::new(a, handler).map_base(|_| ap.into()).unwrap();
        let b = Catcher::new(b, handler).map_base(|_| bp.into()).unwrap();
        a.collides_with(&b)
    }

    #[test]
    fn catcher_collisions() {
        for path in &["/a", "/foo", "/a/b/c", "/a/b/c/d/e"] {
            assert!(catchers_collide(404, path, 404, path));
            assert!(catchers_collide(500, path, 500, path));
            assert!(catchers_collide(None, path, None, path));
        }
    }

    #[test]
    fn catcher_non_collisions() {
        assert!(!catchers_collide(404, "/foo", 405, "/foo"));
        assert!(!catchers_collide(404, "/", None, "/foo"));
        assert!(!catchers_collide(404, "/", None, "/"));
        assert!(!catchers_collide(404, "/a/b", None, "/a/b"));
        assert!(!catchers_collide(404, "/a/b", 404, "/a/b/c"));

        assert!(!catchers_collide(None, "/a/b", None, "/a/b/c"));
        assert!(!catchers_collide(None, "/b", None, "/a/b/c"));
        assert!(!catchers_collide(None, "/", None, "/a/b/c"));
    }
}
