use super::rocket;
use rocket::testing::MockRequest;
use rocket::http::Method::*;

#[test]
fn hello() {
    let rocket = rocket();
    let mut req = MockRequest::new(Get, "/");
    let mut response = req.dispatch_with(&rocket);

    assert_eq!(response.body_string(), Some("Rocketeer".into()));
}
