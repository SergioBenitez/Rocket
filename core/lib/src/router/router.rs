use std::collections::HashMap;

use crate::request::Request;
use crate::http::{Method, Status};

use crate::websocket::WebSocketEvent;
use crate::{Route, Catcher};
use crate::router::Collide;

#[derive(Debug, Default)]
pub(crate) struct Router {
    routes: HashMap<Method, Vec<Route>>,
    catchers: HashMap<Option<u16>, Vec<Catcher>>,
    websocket: HashMap<WebSocketEvent, Vec<Route>>,
}

#[derive(Debug)]
pub struct Collisions {
    pub routes: Vec<(Route, Route)>,
    pub catchers: Vec<(Catcher, Catcher)>,
    pub websocket: Vec<(Route, Route)>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_route(&mut self, route: Route) {
        if let Some(event) = WebSocketEvent::from_handler(&route.websocket_handler) {
            let routes = self.websocket.entry(event).or_default();
            routes.push(route.clone());
            routes.sort_by_key(|r| r.websocket_rank());
        }

        let routes = self.routes.entry(route.method).or_default();
        routes.push(route);
        routes.sort_by_key(|r| r.http_rank());
    }

    pub fn add_catcher(&mut self, catcher: Catcher) {
        let catchers = self.catchers.entry(catcher.code).or_default();
        catchers.push(catcher);
        catchers.sort_by(|a, b| b.base.path().segments().len().cmp(&a.base.path().segments().len()))
    }

    #[inline]
    pub fn routes(&self) -> impl Iterator<Item = &Route> + Clone {
        self.routes.values().flat_map(|v| v.iter())
    }

    #[inline]
    pub fn catchers(&self) -> impl Iterator<Item = &Catcher> + Clone {
        self.catchers.values().flat_map(|v| v.iter())
    }

    #[inline]
    pub fn websocket_events(&self) -> impl Iterator<Item = &Route> + Clone {
        self.websocket.values().flat_map(|v| v.iter())
    }

    pub fn route<'r, 'a: 'r>(
        &'a self,
        req: &'r Request<'r>
    ) -> impl Iterator<Item = &'a Route> + 'r {
        // Note that routes are presorted by ascending rank on each `add`.
        self.routes.get(&req.method())
            .into_iter()
            .flat_map(move |routes| routes.iter().filter(move |r| r.matches(req)))
    }

    pub fn route_event<'r, 'a: 'r>(
        &'a self,
        //req: &'r Request<'r>,
        event: WebSocketEvent,
    ) -> impl Iterator<Item = &'a Route> + 'r {
        // Note that routes are presorted by ascending rank on each `add`.
        self.websocket.get(&event)
            .into_iter()
            .flat_map(move |routes| routes)
    }

    // For many catchers, using aho-corasick or similar should be much faster.
    pub fn catch<'r>(&self, status: Status, req: &'r Request<'r>) -> Option<&Catcher> {
        // Note that catchers are presorted by descending base length.
        let explicit = self.catchers.get(&Some(status.code))
            .and_then(|c| c.iter().find(|c| c.matches(status, req)));

        let default = self.catchers.get(&None)
            .and_then(|c| c.iter().find(|c| c.matches(status, req)));

        match (explicit, default) {
            (None, None) => None,
            (None, c@Some(_)) | (c@Some(_), None) => c,
            (Some(a), Some(b)) => {
                if b.base.path().segments().len() > a.base.path().segments().len() {
                    Some(b)
                } else {
                    Some(a)
                }
            }
        }
    }

    fn collisions<'a, I, T>(&self, items: I) -> impl Iterator<Item = (T, T)> + 'a
        where I: Iterator<Item = &'a T> + Clone + 'a, T: Collide + Clone + 'a,
    {
        items.clone().enumerate()
            .flat_map(move |(i, a)| {
                items.clone()
                    .skip(i + 1)
                    .filter(move |b| a.collides_with(b))
                    .map(move |b| (a.clone(), b.clone()))
            })
    }

    pub fn finalize(&self) -> Result<(), Collisions> {
        let routes: Vec<_> = self.collisions(self.routes()).collect();
        let catchers: Vec<_> = self.collisions(self.catchers()).collect();
        let websocket: Vec<_> = self.collisions(self.websocket_events()).collect();

        if !routes.is_empty() || !catchers.is_empty() {
            return Err(Collisions { routes, catchers, websocket })
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::route::dummy_handler;
    use crate::local::blocking::Client;
    use crate::http::{Method, Method::*, uri::Origin};

    impl Router {
        fn has_collisions(&self) -> bool {
            self.finalize().is_err()
        }
    }

    fn router_with_routes(routes: &[&'static str]) -> Router {
        let mut router = Router::new();
        for route in routes {
            let route = Route::new(Get, route, dummy_handler);
            router.add_route(route);
        }

        router
    }

    fn router_with_ranked_routes(routes: &[(isize, &'static str)]) -> Router {
        let mut router = Router::new();
        for &(rank, route) in routes {
            let route = Route::ranked(rank, Get, route, dummy_handler);
            router.add_route(route);
        }

        router
    }

    fn router_with_rankless_routes(routes: &[&'static str]) -> Router {
        let mut router = Router::new();
        for route in routes {
            let route = Route::ranked(0, Get, route, dummy_handler);
            router.add_route(route);
        }

        router
    }

    fn rankless_route_collisions(routes: &[&'static str]) -> bool {
        let router = router_with_rankless_routes(routes);
        router.has_collisions()
    }

    fn default_rank_route_collisions(routes: &[&'static str]) -> bool {
        let router = router_with_routes(routes);
        router.has_collisions()
    }

    #[test]
    fn test_rankless_collisions() {
        assert!(rankless_route_collisions(&["/hello", "/hello"]));
        assert!(rankless_route_collisions(&["/<a>", "/hello"]));
        assert!(rankless_route_collisions(&["/<a>", "/<b>"]));
        assert!(rankless_route_collisions(&["/hello/bob", "/hello/<b>"]));
        assert!(rankless_route_collisions(&["/a/b/<c>/d", "/<a>/<b>/c/d"]));

        assert!(rankless_route_collisions(&["/a/b", "/<a..>"]));
        assert!(rankless_route_collisions(&["/a/b/c", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["/<a>/b", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["/a/<b>", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["/a/b/<c>", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["/<a..>", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["/a/<a..>", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["/a/b/<a..>", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["/a/b/c/d", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["/", "/<a..>"]));
        assert!(rankless_route_collisions(&["/a/<_>", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["/a/<_>", "/a/<_..>"]));
        assert!(rankless_route_collisions(&["/<_>", "/a/<_..>"]));
        assert!(rankless_route_collisions(&["/foo", "/foo/<_..>"]));
        assert!(rankless_route_collisions(&["/foo/bar/baz", "/foo/<_..>"]));
        assert!(rankless_route_collisions(&["/a/d/<b..>", "/a/d"]));
        assert!(rankless_route_collisions(&["/a/<_..>", "/<_>"]));
        assert!(rankless_route_collisions(&["/a/<_..>", "/a"]));
        assert!(rankless_route_collisions(&["/<a>", "/a/<a..>"]));

        assert!(rankless_route_collisions(&["/<_>", "/<_>"]));
        assert!(rankless_route_collisions(&["/a/<_>", "/a/b"]));
        assert!(rankless_route_collisions(&["/a/<_>", "/a/<b>"]));
        assert!(rankless_route_collisions(&["/<_..>", "/a/b"]));
        assert!(rankless_route_collisions(&["/<_..>", "/<_>"]));
        assert!(rankless_route_collisions(&["/<_>/b", "/a/b"]));
        assert!(rankless_route_collisions(&["/", "/<foo..>"]));
    }

    #[test]
    fn test_collisions_normalize() {
        assert!(rankless_route_collisions(&["/hello/", "/hello"]));
        assert!(rankless_route_collisions(&["//hello/", "/hello"]));
        assert!(rankless_route_collisions(&["//hello/", "/hello//"]));
        assert!(rankless_route_collisions(&["/<a>", "/hello//"]));
        assert!(rankless_route_collisions(&["/<a>", "/hello///"]));
        assert!(rankless_route_collisions(&["/hello///bob", "/hello/<b>"]));
        assert!(rankless_route_collisions(&["/<a..>//", "/a//<a..>"]));
        assert!(rankless_route_collisions(&["/a/<a..>//", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["/a/<a..>//", "/a/b//c//d/"]));
        assert!(rankless_route_collisions(&["/a/<a..>/", "/a/bd/e/"]));
        assert!(rankless_route_collisions(&["/<a..>/", "/a/bd/e/"]));
        assert!(rankless_route_collisions(&["//", "/<foo..>"]));
        assert!(rankless_route_collisions(&["/a/<a..>//", "/a/b//c//d/e/"]));
        assert!(rankless_route_collisions(&["/a//<a..>//", "/a/b//c//d/e/"]));
        assert!(rankless_route_collisions(&["///<_>", "/<_>"]));
        assert!(rankless_route_collisions(&["/a/<_>", "///a//b"]));
        assert!(rankless_route_collisions(&["//a///<_>", "/a//<b>"]));
        assert!(rankless_route_collisions(&["//<_..>", "/a/b"]));
        assert!(rankless_route_collisions(&["//<_..>", "/<_>"]));
        assert!(rankless_route_collisions(&["///<a>/", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["///<a..>/", "/a/<a..>"]));
        assert!(rankless_route_collisions(&["/<a..>", "/hello"]));
    }

    #[test]
    fn test_collisions_query() {
        // Query shouldn't affect things when rankless.
        assert!(rankless_route_collisions(&["/hello?<foo>", "/hello"]));
        assert!(rankless_route_collisions(&["/<a>?foo=bar", "/hello?foo=bar&cat=fat"]));
        assert!(rankless_route_collisions(&["/<a>?foo=bar", "/hello?foo=bar&cat=fat"]));
        assert!(rankless_route_collisions(&["/<a>", "/<b>?<foo>"]));
        assert!(rankless_route_collisions(&["/hello/bob?a=b", "/hello/<b>?d=e"]));
        assert!(rankless_route_collisions(&["/<foo>?a=b", "/foo?d=e"]));
        assert!(rankless_route_collisions(&["/<foo>?a=b&<c>", "/<foo>?d=e&<c>"]));
        assert!(rankless_route_collisions(&["/<foo>?a=b&<c>", "/<foo>?d=e"]));
    }

    #[test]
    fn test_no_collisions() {
        assert!(!rankless_route_collisions(&["/a/b", "/a/b/c"]));
        assert!(!rankless_route_collisions(&["/a/b/c/d", "/a/b/c/<d>/e"]));
        assert!(!rankless_route_collisions(&["/a/d/<b..>", "/a/b/c"]));
        assert!(!rankless_route_collisions(&["/<_>", "/"]));
        assert!(!rankless_route_collisions(&["/a/<_>", "/a"]));
        assert!(!rankless_route_collisions(&["/a/<_>", "/<_>"]));
    }

    #[test]
    fn test_no_collision_when_ranked() {
        assert!(!default_rank_route_collisions(&["/<a>", "/hello"]));
        assert!(!default_rank_route_collisions(&["/hello/bob", "/hello/<b>"]));
        assert!(!default_rank_route_collisions(&["/a/b/c/d", "/<a>/<b>/c/d"]));
        assert!(!default_rank_route_collisions(&["/hi", "/<hi>"]));
        assert!(!default_rank_route_collisions(&["/a", "/a/<path..>"]));
        assert!(!default_rank_route_collisions(&["/", "/<path..>"]));
        assert!(!default_rank_route_collisions(&["/a/b", "/a/b/<c..>"]));
        assert!(!default_rank_route_collisions(&["/<_>", "/static"]));
        assert!(!default_rank_route_collisions(&["/<_..>", "/static"]));
        assert!(!default_rank_route_collisions(&["/<path..>", "/"]));
        assert!(!default_rank_route_collisions(&["/<_>/<_>", "/foo/bar"]));
        assert!(!default_rank_route_collisions(&["/foo/<_>", "/foo/bar"]));

        assert!(!default_rank_route_collisions(&["/<a>/<b>", "/hello/<b>"]));
        assert!(!default_rank_route_collisions(&["/<a>/<b..>", "/hello/<b>"]));
        assert!(!default_rank_route_collisions(&["/<a..>", "/hello/<b>"]));
        assert!(!default_rank_route_collisions(&["/<a..>", "/hello"]));
        assert!(!default_rank_route_collisions(&["/<a>", "/a/<path..>"]));
        assert!(!default_rank_route_collisions(&["/a/<b>/c", "/<d>/<c..>"]));
    }

    #[test]
    fn test_collision_when_ranked() {
        assert!(default_rank_route_collisions(&["/a/<b>/<c..>", "/a/<c>"]));
        assert!(default_rank_route_collisions(&["/<a>/b", "/a/<b>"]));
    }

    #[test]
    fn test_collision_when_ranked_query() {
        assert!(default_rank_route_collisions(&["/a?a=b", "/a?c=d"]));
        assert!(default_rank_route_collisions(&["/a?a=b&<b>", "/a?<c>&c=d"]));
        assert!(default_rank_route_collisions(&["/a?a=b&<b..>", "/a?<c>&c=d"]));
    }

    #[test]
    fn test_no_collision_when_ranked_query() {
        assert!(!default_rank_route_collisions(&["/", "/?<c..>"]));
        assert!(!default_rank_route_collisions(&["/hi", "/hi?<c>"]));
        assert!(!default_rank_route_collisions(&["/hi", "/hi?c"]));
        assert!(!default_rank_route_collisions(&["/hi?<c>", "/hi?c"]));
        assert!(!default_rank_route_collisions(&["/<foo>?a=b", "/<foo>?c=d&<d>"]));
    }

    fn matches<'a>(router: &'a Router, method: Method, uri: &'a str) -> Vec<&'a Route> {
        let client = Client::debug_with(vec![]).expect("client");
        let request = client.req(method, Origin::parse(uri).unwrap());
        router.route(&request).collect()
    }

    fn route<'a>(router: &'a Router, method: Method, uri: &'a str) -> Option<&'a Route> {
        matches(router, method, uri).into_iter().next()
    }

    #[test]
    fn test_ok_routing() {
        let router = router_with_routes(&["/hello"]);
        assert!(route(&router, Get, "/hello").is_some());

        let router = router_with_routes(&["/<a>"]);
        assert!(route(&router, Get, "/hello").is_some());
        assert!(route(&router, Get, "/hi").is_some());
        assert!(route(&router, Get, "/bobbbbbbbbbby").is_some());
        assert!(route(&router, Get, "/dsfhjasdf").is_some());

        let router = router_with_routes(&["/<a>/<b>"]);
        assert!(route(&router, Get, "/hello/hi").is_some());
        assert!(route(&router, Get, "/a/b/").is_some());
        assert!(route(&router, Get, "/i/a").is_some());
        assert!(route(&router, Get, "/jdlk/asdij").is_some());

        let mut router = Router::new();
        router.add_route(Route::new(Put, "/hello", dummy_handler));
        router.add_route(Route::new(Post, "/hello", dummy_handler));
        router.add_route(Route::new(Delete, "/hello", dummy_handler));
        assert!(route(&router, Put, "/hello").is_some());
        assert!(route(&router, Post, "/hello").is_some());
        assert!(route(&router, Delete, "/hello").is_some());

        let router = router_with_routes(&["/<a..>"]);
        assert!(route(&router, Get, "/").is_some());
        assert!(route(&router, Get, "//").is_some());
        assert!(route(&router, Get, "/hi").is_some());
        assert!(route(&router, Get, "/hello/hi").is_some());
        assert!(route(&router, Get, "/a/b/").is_some());
        assert!(route(&router, Get, "/i/a").is_some());
        assert!(route(&router, Get, "/a/b/c/d/e/f").is_some());

        let router = router_with_routes(&["/foo/<a..>"]);
        assert!(route(&router, Get, "/foo").is_some());
        assert!(route(&router, Get, "/foo/").is_some());
        assert!(route(&router, Get, "/foo///bar").is_some());
    }

    #[test]
    fn test_err_routing() {
        let router = router_with_routes(&["/hello"]);
        assert!(route(&router, Put, "/hello").is_none());
        assert!(route(&router, Post, "/hello").is_none());
        assert!(route(&router, Options, "/hello").is_none());
        assert!(route(&router, Get, "/hell").is_none());
        assert!(route(&router, Get, "/hi").is_none());
        assert!(route(&router, Get, "/hello/there").is_none());
        assert!(route(&router, Get, "/hello/i").is_none());
        assert!(route(&router, Get, "/hillo").is_none());

        let router = router_with_routes(&["/<a>"]);
        assert!(route(&router, Put, "/hello").is_none());
        assert!(route(&router, Post, "/hello").is_none());
        assert!(route(&router, Options, "/hello").is_none());
        assert!(route(&router, Get, "/hello/there").is_none());
        assert!(route(&router, Get, "/hello/i").is_none());

        let router = router_with_routes(&["/<a>/<b>"]);
        assert!(route(&router, Get, "/a/b/c").is_none());
        assert!(route(&router, Get, "/a").is_none());
        assert!(route(&router, Get, "/a/").is_none());
        assert!(route(&router, Get, "/a/b/c/d").is_none());
        assert!(route(&router, Put, "/hello/hi").is_none());
        assert!(route(&router, Put, "/a/b").is_none());
        assert!(route(&router, Put, "/a/b").is_none());

        let router = router_with_routes(&["/prefix/<a..>"]);
        assert!(route(&router, Get, "/").is_none());
        assert!(route(&router, Get, "/prefi/").is_none());
    }

    macro_rules! assert_ranked_match {
        ($routes:expr, $to:expr => $want:expr) => ({
            let router = router_with_routes($routes);
            assert!(!router.has_collisions());
            let route_path = route(&router, Get, $to).unwrap().uri.to_string();
            assert_eq!(route_path, $want.to_string());
        })
    }

    #[test]
    fn test_default_ranking() {
        assert_ranked_match!(&["/hello", "/<name>"], "/hello" => "/hello");
        assert_ranked_match!(&["/<name>", "/hello"], "/hello" => "/hello");
        assert_ranked_match!(&["/<a>", "/hi", "/hi/<b>"], "/hi" => "/hi");
        assert_ranked_match!(&["/<a>/b", "/hi/c"], "/hi/c" => "/hi/c");
        assert_ranked_match!(&["/<a>/<b>", "/hi/a"], "/hi/c" => "/<a>/<b>");
        assert_ranked_match!(&["/hi/a", "/hi/<c>"], "/hi/c" => "/hi/<c>");
        assert_ranked_match!(&["/a", "/a?<b>"], "/a?b=c" => "/a?<b>");
        assert_ranked_match!(&["/a", "/a?<b>"], "/a" => "/a?<b>");
        assert_ranked_match!(&["/a", "/<a>", "/a?<b>", "/<a>?<b>"], "/a" => "/a?<b>");
        assert_ranked_match!(&["/a", "/<a>", "/a?<b>", "/<a>?<b>"], "/b" => "/<a>?<b>");
        assert_ranked_match!(&["/a", "/<a>", "/a?<b>", "/<a>?<b>"], "/b?v=1" => "/<a>?<b>");
        assert_ranked_match!(&["/a", "/<a>", "/a?<b>", "/<a>?<b>"], "/a?b=c" => "/a?<b>");
        assert_ranked_match!(&["/a", "/a?b"], "/a?b" => "/a?b");
        assert_ranked_match!(&["/<a>", "/a?b"], "/a?b" => "/a?b");
        assert_ranked_match!(&["/a", "/<a>?b"], "/a?b" => "/a");
        assert_ranked_match!(&["/a?<c>&b", "/a?<b>"], "/a" => "/a?<b>");
        assert_ranked_match!(&["/a?<c>&b", "/a?<b>"], "/a?b" => "/a?<c>&b");
        assert_ranked_match!(&["/a?<c>&b", "/a?<b>"], "/a?c" => "/a?<b>");
        assert_ranked_match!(&["/", "/<foo..>"], "/" => "/");
        assert_ranked_match!(&["/", "/<foo..>"], "/hi" => "/<foo..>");
        assert_ranked_match!(&["/hi", "/<foo..>"], "/hi" => "/hi");
    }

    fn ranked_collisions(routes: &[(isize, &'static str)]) -> bool {
        let router = router_with_ranked_routes(routes);
        router.has_collisions()
    }

    #[test]
    fn test_no_manual_ranked_collisions() {
        assert!(!ranked_collisions(&[(1, "/a/<b>"), (2, "/a/<b>")]));
        assert!(!ranked_collisions(&[(0, "/a/<b>"), (2, "/a/<b>")]));
        assert!(!ranked_collisions(&[(5, "/a/<b>"), (2, "/a/<b>")]));
        assert!(!ranked_collisions(&[(1, "/a/<b>"), (1, "/b/<b>")]));
        assert!(!ranked_collisions(&[(1, "/a/<b..>"), (2, "/a/<b..>")]));
        assert!(!ranked_collisions(&[(0, "/a/<b..>"), (2, "/a/<b..>")]));
        assert!(!ranked_collisions(&[(5, "/a/<b..>"), (2, "/a/<b..>")]));
        assert!(!ranked_collisions(&[(1, "/<a..>"), (2, "/<a..>")]));
    }

    #[test]
    fn test_ranked_collisions() {
        assert!(ranked_collisions(&[(2, "/a/<b..>"), (2, "/a/<b..>")]));
        assert!(ranked_collisions(&[(2, "/a/c/<b..>"), (2, "/a/<b..>")]));
        assert!(ranked_collisions(&[(2, "/<b..>"), (2, "/a/<b..>")]));
    }

    macro_rules! assert_ranked_routing {
        (to: $to:expr, with: $routes:expr, expect: $($want:expr),+) => ({
            let router = router_with_ranked_routes(&$routes);
            let routed_to = matches(&router, Get, $to);
            let expected = &[$($want),+];
            assert!(routed_to.len() == expected.len());
            for (got, expected) in routed_to.iter().zip(expected.iter()) {
                assert_eq!(got.rank, expected.0);
                assert_eq!(got.uri.to_string(), expected.1.to_string());
            }
        })
    }

    #[test]
    fn test_ranked_routing() {
        assert_ranked_routing!(
            to: "/a/b",
            with: [(1, "/a/<b>"), (2, "/a/<b>")],
            expect: (1, "/a/<b>"), (2, "/a/<b>")
        );

        assert_ranked_routing!(
            to: "/b/b",
            with: [(1, "/a/<b>"), (2, "/b/<b>"), (3, "/b/b")],
            expect: (2, "/b/<b>"), (3, "/b/b")
        );

        assert_ranked_routing!(
            to: "/b/b",
            with: [(2, "/b/<b>"), (1, "/a/<b>"), (3, "/b/b")],
            expect: (2, "/b/<b>"), (3, "/b/b")
        );

        assert_ranked_routing!(
            to: "/b/b",
            with: [(3, "/b/b"), (2, "/b/<b>"), (1, "/a/<b>")],
            expect: (2, "/b/<b>"), (3, "/b/b")
        );

        assert_ranked_routing!(
            to: "/b/b",
            with: [(1, "/a/<b>"), (2, "/b/<b>"), (0, "/b/b")],
            expect: (0, "/b/b"), (2, "/b/<b>")
        );

        assert_ranked_routing!(
            to: "/profile/sergio/edit",
            with: [(1, "/<a>/<b>/edit"), (2, "/profile/<d>"), (0, "/<a>/<b>/<c>")],
            expect: (0, "/<a>/<b>/<c>"), (1, "/<a>/<b>/edit")
        );

        assert_ranked_routing!(
            to: "/profile/sergio/edit",
            with: [(0, "/<a>/<b>/edit"), (2, "/profile/<d>"), (5, "/<a>/<b>/<c>")],
            expect: (0, "/<a>/<b>/edit"), (5, "/<a>/<b>/<c>")
        );

        assert_ranked_routing!(
            to: "/a/b",
            with: [(0, "/a/b"), (1, "/a/<b..>")],
            expect: (0, "/a/b"), (1, "/a/<b..>")
        );

        assert_ranked_routing!(
            to: "/a/b/c/d/e/f",
            with: [(1, "/a/<b..>"), (2, "/a/b/<c..>")],
            expect: (1, "/a/<b..>"), (2, "/a/b/<c..>")
        );

        assert_ranked_routing!(
            to: "/hi",
            with: [(1, "/hi/<foo..>"), (0, "/hi/<foo>")],
            expect: (1, "/hi/<foo..>")
        );
    }

    macro_rules! assert_default_ranked_routing {
        (to: $to:expr, with: $routes:expr, expect: $($want:expr),+) => ({
            let router = router_with_routes(&$routes);
            let routed_to = matches(&router, Get, $to);
            let expected = &[$($want),+];
            assert!(routed_to.len() == expected.len());
            for (got, expected) in routed_to.iter().zip(expected.iter()) {
                assert_eq!(got.uri.to_string(), expected.to_string());
            }
        })
    }

    #[test]
    fn test_default_ranked_routing() {
        assert_default_ranked_routing!(
            to: "/a/b?v=1",
            with: ["/a/<b>", "/a/b"],
            expect: "/a/b", "/a/<b>"
        );

        assert_default_ranked_routing!(
            to: "/a/b?v=1",
            with: ["/a/<b>", "/a/b", "/a/b?<v>"],
            expect: "/a/b?<v>", "/a/b", "/a/<b>"
        );

        assert_default_ranked_routing!(
            to: "/a/b?v=1",
            with: ["/a/<b>", "/a/b", "/a/b?<v>", "/a/<b>?<v>"],
            expect: "/a/b?<v>", "/a/b", "/a/<b>?<v>", "/a/<b>"
        );

        assert_default_ranked_routing!(
            to: "/a/b",
            with: ["/a/<b>", "/a/b", "/a/b?<v>", "/a/<b>?<v>"],
            expect: "/a/b?<v>", "/a/b", "/a/<b>?<v>", "/a/<b>"
        );

        assert_default_ranked_routing!(
            to: "/a/b?c",
            with: ["/a/b", "/a/b?<c>", "/a/b?c", "/a/<b>?c", "/a/<b>?<c>", "/<a>/<b>"],
            expect: "/a/b?c", "/a/b?<c>", "/a/b", "/a/<b>?c", "/a/<b>?<c>", "/<a>/<b>"
        );
    }

    fn router_with_catchers(catchers: &[(Option<u16>, &str)]) -> Router {
        let mut router = Router::new();
        for (code, base) in catchers {
            let catcher = Catcher::new(*code, crate::catcher::dummy_handler);
            router.add_catcher(catcher.map_base(|_| base.to_string()).unwrap());
        }

        router
    }

    fn catcher<'a>(router: &'a Router, status: Status, uri: &str) -> Option<&'a Catcher> {
        let client = Client::debug_with(vec![]).expect("client");
        let request = client.get(Origin::parse(uri).unwrap());
        router.catch(status, &request)
    }

    macro_rules! assert_catcher_routing {
        (
            catch: [$(($code:expr, $uri:expr)),+],
            reqs: [$($r:expr),+],
            with: [$(($ecode:expr, $euri:expr)),+]
        ) => ({
            let catchers = vec![$(($code.into(), $uri)),+];
            let requests = vec![$($r),+];
            let expected = vec![$(($ecode.into(), $euri)),+];

            let router = router_with_catchers(&catchers);
            for (req, expected) in requests.iter().zip(expected.iter()) {
                let req_status = Status::from_code(req.0).expect("valid status");
                let catcher = catcher(&router, req_status, req.1).expect("some catcher");
                assert_eq!(catcher.code, expected.0, "<- got, expected ->");
                assert_eq!(catcher.base.path(), expected.1, "<- got, expected ->");
            }
        })
    }

    #[test]
    fn test_catcher_routing() {
        // Check that the default `/` catcher catches everything.
        assert_catcher_routing! {
            catch: [(None, "/")],
            reqs: [(404, "/a/b/c"), (500, "/a/b"), (415, "/a/b/d"), (422, "/a/b/c/d?foo")],
            with: [(None, "/"), (None, "/"), (None, "/"), (None, "/")]
        }

        // Check prefixes when they're exact.
        assert_catcher_routing! {
            catch: [(None, "/"), (None, "/a"), (None, "/a/b")],
            reqs: [
                (404, "/"), (500, "/"),
                (404, "/a"), (500, "/a"),
                (404, "/a/b"), (500, "/a/b")
            ],
            with: [
                (None, "/"), (None, "/"),
                (None, "/a"), (None, "/a"),
                (None, "/a/b"), (None, "/a/b")
            ]
        }

        // Check prefixes when they're not exact.
        assert_catcher_routing! {
            catch: [(None, "/"), (None, "/a"), (None, "/a/b")],
            reqs: [
                (404, "/foo"), (500, "/bar"), (422, "/baz/bar"), (418, "/poodle?yes"),
                (404, "/a/foo"), (500, "/a/bar/baz"), (510, "/a/c"), (423, "/a/c/b"),
                (404, "/a/b/c"), (500, "/a/b/c/d"), (500, "/a/b?foo"), (400, "/a/b/yes")
            ],
            with: [
                (None, "/"), (None, "/"), (None, "/"), (None, "/"),
                (None, "/a"), (None, "/a"), (None, "/a"), (None, "/a"),
                (None, "/a/b"), (None, "/a/b"), (None, "/a/b"), (None, "/a/b")
            ]
        }

        // Check that we prefer specific to default.
        assert_catcher_routing! {
            catch: [(400, "/"), (404, "/"), (None, "/")],
            reqs: [
                (400, "/"), (400, "/bar"), (400, "/foo/bar"),
                (404, "/"), (404, "/bar"), (404, "/foo/bar"),
                (405, "/"), (405, "/bar"), (406, "/foo/bar")
            ],
            with: [
                (400, "/"), (400, "/"), (400, "/"),
                (404, "/"), (404, "/"), (404, "/"),
                (None, "/"), (None, "/"), (None, "/")
            ]
        }

        // Check that we prefer longer prefixes over specific.
        assert_catcher_routing! {
            catch: [(None, "/a/b"), (404, "/a"), (422, "/a")],
            reqs: [
                (404, "/a/b"), (404, "/a/b/c"), (422, "/a/b/c"),
                (404, "/a"), (404, "/a/c"), (404, "/a/cat/bar"),
                (422, "/a"), (422, "/a/c"), (422, "/a/cat/bar")
            ],
            with: [
                (None, "/a/b"), (None, "/a/b"), (None, "/a/b"),
                (404, "/a"), (404, "/a"), (404, "/a"),
                (422, "/a"), (422, "/a"), (422, "/a")
            ]
        }

        // Just a fun one.
        assert_catcher_routing! {
            catch: [(None, "/"), (None, "/a/b"), (500, "/a/b/c"), (500, "/a/b")],
            reqs: [(404, "/a/b/c"), (500, "/a/b"), (400, "/a/b/d"), (500, "/a/b/c/d?foo")],
            with: [(None, "/a/b"), (500, "/a/b"), (None, "/a/b"), (500, "/a/b/c")]
        }
    }
}
