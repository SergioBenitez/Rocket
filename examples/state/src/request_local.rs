use std::sync::atomic::{AtomicUsize, Ordering};

use rocket::State;
use rocket::outcome::Outcome;
use rocket::request::{self, FromRequest, Request};
use rocket::fairing::AdHoc;
use rocket::trace::info;

#[derive(Default, Debug)]
pub struct Atomics {
    pub uncached: AtomicUsize,
    pub cached: AtomicUsize,
}

struct Guard1;
struct Guard2;
struct Guard3;
struct Guard4;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Guard1 {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, ()> {
        info!("-- 1 --");

        let atomics = try_outcome!(req.guard::<State<'_, Atomics>>().await);
        atomics.uncached.fetch_add(1, Ordering::Relaxed);
        req.local_cache(|| {
            info!("1: populating cache!");
            atomics.cached.fetch_add(1, Ordering::Relaxed)
        });

        Outcome::Success(Guard1)
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Guard2 {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, ()> {
        info!("-- 2 --");

        try_outcome!(req.guard::<Guard1>().await);
        Outcome::Success(Guard2)
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Guard3 {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, ()> {
        info!("-- 3 --");

        let atomics = try_outcome!(req.guard::<State<'_, Atomics>>().await);
        atomics.uncached.fetch_add(1, Ordering::Relaxed);
        req.local_cache_async(async {
            info!("3: populating cache!");
            atomics.cached.fetch_add(1, Ordering::Relaxed)
        }).await;

        Outcome::Success(Guard3)
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Guard4 {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, ()> {
        info!("-- 4 --");

        try_outcome!(Guard3::from_request(req).await);
        Outcome::Success(Guard4)
    }
}

#[get("/1-2")]
fn one_two(_g1: Guard1, _g2: Guard2, state: State<'_, Atomics>) -> String {
    format!("{:#?}", state)
}

#[get("/3-4")]
fn three_four(_g3: Guard3, _g4: Guard4, state: State<'_, Atomics>) -> String {
    format!("{:#?}", state)
}

#[get("/1-2-3-4")]
fn all(
    _g1: Guard1,
    _g2: Guard2,
    _g3: Guard3,
    _g4: Guard4,
    state: State<'_, Atomics>
) -> String {
    format!("{:#?}", state)
}

pub fn stage() -> AdHoc {
    AdHoc::on_ignite("Request Local State", |rocket| async {
        rocket.manage(Atomics::default())
            .mount("/req-local", routes![one_two, three_four, all])
    })
}
