#![feature(plugin)]
#![plugin(rocket_codegen)]

extern crate rocket;

#[post("/", format = "application/json")]
fn specified() -> &'static str {
    "specified"
}

#[post("/", rank = 2)]
fn unspecified() -> &'static str {
    "unspecified"
}

#[post("/", format = "application/json")]
fn specified_json() -> &'static str {
    "specified_json"
}

#[post("/", format = "text/html")]
fn specified_html() -> &'static str {
    "specified_html"
}

#[cfg(feature = "testing")]
mod tests {
    use super::*;

    use rocket::Rocket;
    use rocket::testing::MockRequest;
    use rocket::http::Method::*;
    use rocket::http::{Status, ContentType};

    fn rocket() -> Rocket {
        rocket::ignite()
            .mount("/first", routes![specified, unspecified])
            .mount("/second", routes![specified_json, specified_html])
    }

    macro_rules! check_dispatch {
        ($mount:expr, $ct:expr, $body:expr) => (
            let rocket = rocket();
            let mut req = MockRequest::new(Post, $mount);
            let ct: Option<ContentType> = $ct;
            if let Some(ct) = ct {
                req.add_header(ct);
            }

            let mut response = req.dispatch_with(&rocket);
            let body_str = response.body().and_then(|b| b.into_string());
            let body: Option<&'static str> = $body;
            match body {
                Some(string) => assert_eq!(body_str, Some(string.to_string())),
                None => assert_eq!(response.status(), Status::NotFound)
            }
        )
    }

    #[test]
    fn exact_match_or_forward() {
        check_dispatch!("/first", Some(ContentType::Json), Some("specified"));
        check_dispatch!("/first", None, Some("unspecified"));
        check_dispatch!("/first", Some(ContentType::HTML), Some("unspecified"));
    }

    #[test]
    fn exact_match_or_none() {
        check_dispatch!("/second", Some(ContentType::Json), Some("specified_json"));
        check_dispatch!("/second", Some(ContentType::HTML), Some("specified_html"));
        check_dispatch!("/second", Some(ContentType::CSV), None);
        check_dispatch!("/second", None, None);
    }
}
