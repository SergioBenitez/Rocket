#[cfg(test)]
mod tests;

use std::env;

use rocket::{Request, Route, Catcher, route, catcher};
use rocket::data::{Data, ToByteUnit};
use rocket::http::{Status, Method::*};
use rocket::response::{Responder, status::Custom};
use rocket::outcome::{try_outcome, IntoOutcome};
use rocket::tokio::fs::File;
use rocket::trace::info;

fn forward<'r>(_req: &'r Request, data: Data) -> route::BoxFuture<'r> {
    Box::pin(async move { route::Outcome::forward(data) })
}

fn hi<'r>(req: &'r Request, _: Data) -> route::BoxFuture<'r> {
    route::Outcome::from(req, "Hello!").pin()
}

fn name<'a>(req: &'a Request, _: Data) -> route::BoxFuture<'a> {
    let param = req.param::<&'a str>(0)
        .and_then(|res| res.ok())
        .unwrap_or("unnamed".into());

    route::Outcome::from(req, param).pin()
}

fn echo_url<'r>(req: &'r Request, _: Data) -> route::BoxFuture<'r> {
    let param_outcome = req.param::<&str>(1)
        .and_then(|res| res.ok())
        .into_outcome(Status::BadRequest);

    Box::pin(async move {
        route::Outcome::from(req, try_outcome!(param_outcome))
    })
}

fn upload<'r>(req: &'r Request, data: Data) -> route::BoxFuture<'r> {
    Box::pin(async move {
        if !req.content_type().map_or(false, |ct| ct.is_plain()) {
            info!("    => Content-Type of upload must be text/plain. Ignoring.");
            return route::Outcome::failure(Status::BadRequest);
        }

        match File::create(env::temp_dir().join("upload.txt")).await {
            Ok(file) => {
                if let Ok(n) = data.open(2.mebibytes()).stream_to(file).await {
                    return route::Outcome::from(req, format!("OK: {} bytes uploaded.", n));
                }

                info!("    => Failed copying.");
                route::Outcome::failure(Status::InternalServerError)
            }
            Err(error) => {
                info!(?error, "    => Couldn't open file");
                route::Outcome::failure(Status::InternalServerError)
            }
        }
    })
}

fn get_upload<'r>(req: &'r Request, _: Data) -> route::BoxFuture<'r> {
    route::Outcome::from(req, std::fs::File::open(env::temp_dir().join("upload.txt")).ok()).pin()
}

fn not_found_handler<'r>(_: Status, req: &'r Request) -> catcher::BoxFuture<'r> {
    let res = Custom(Status::NotFound, format!("Couldn't find: {}", req.uri()));
    Box::pin(async move { res.respond_to(req) })
}

#[derive(Clone)]
struct CustomHandler {
    data: &'static str
}

impl CustomHandler {
    fn new(data: &'static str) -> Vec<Route> {
        vec![Route::new(Get, "/<id>", Self { data })]
    }
}

#[rocket::async_trait]
impl route::Handler for CustomHandler {
    async fn handle<'r, 's: 'r>(&'s self, req: &'r Request<'_>, data: Data) -> route::Outcome<'r> {
        let self_data = self.data;
        let id = req.param::<&str>(0)
            .and_then(|res| res.ok())
            .or_forward(data);

        route::Outcome::from(req, format!("{} - {}", self_data, try_outcome!(id)))
    }
}

#[rocket::launch]
fn rocket() -> _ {
    let always_forward = Route::ranked(1, Get, "/", forward);
    let hello = Route::ranked(2, Get, "/", hi);

    let echo = Route::new(Get, "/echo/<str>", echo_url);
    let name = Route::new(Get, "/<name>", name);
    let post_upload = Route::new(Post, "/", upload);
    let get_upload = Route::new(Get, "/", get_upload);

    let not_found_catcher = Catcher::new(404, not_found_handler);

    rocket::build()
        .mount("/", vec![always_forward, hello, echo])
        .mount("/upload", vec![get_upload, post_upload])
        .mount("/hello", vec![name.clone()])
        .mount("/hi", vec![name])
        .mount("/custom", CustomHandler::new("some data here"))
        .register("/", vec![not_found_catcher])
}
