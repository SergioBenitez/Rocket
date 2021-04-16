use crate::{Rocket, Request, Response, Data, Build, Orbit};
use crate::fairing::{Fairing, Info, Kind};
use crate::trace::PaintExt;

use yansi::Paint;

#[derive(Default)]
pub struct Fairings {
    all_fairings: Vec<Box<dyn Fairing>>,
    failures: Vec<Info>,
    // Index into `attach` of last run attach fairing.
    last_launch: usize,
    // The vectors below hold indices into `all_fairings`.
    launch: Vec<usize>,
    liftoff: Vec<usize>,
    request: Vec<usize>,
    response: Vec<usize>,
}

macro_rules! iter {
    ($_self:ident . $kind:ident) => ({
        let all_fairings = &$_self.all_fairings;
        $_self.$kind.iter().filter_map(move |i| all_fairings.get(*i).map(|f| &**f))
    })
}

impl Fairings {
    #[inline]
    pub fn new() -> Fairings {
        Fairings::default()
    }

    pub fn add(&mut self, fairing: Box<dyn Fairing>) -> &dyn Fairing {
        let kind = fairing.info().kind;
        let index = self.all_fairings.len();
        self.all_fairings.push(fairing);

        if kind.is(Kind::Ignite) { self.launch.push(index); }
        if kind.is(Kind::Liftoff) { self.liftoff.push(index); }
        if kind.is(Kind::Request) { self.request.push(index); }
        if kind.is(Kind::Response) { self.response.push(index); }

        &*self.all_fairings[index]
    }

    pub fn append(&mut self, others: &mut Fairings) {
        for fairing in others.all_fairings.drain(..) {
            self.add(fairing);
        }
    }

    pub async fn handle_ignite(mut rocket: Rocket<Build>) -> Rocket<Build> {
        while rocket.fairings.last_launch < rocket.fairings.launch.len() {
            // We're going to move `rocket` while borrowing `fairings`...
            let mut fairings = std::mem::replace(&mut rocket.fairings, Fairings::new());
            for fairing in iter!(fairings.launch).skip(fairings.last_launch) {
                let info = fairing.info();
                rocket = match fairing.on_ignite(rocket).await {
                    Ok(rocket) => rocket,
                    Err(rocket) => {
                        fairings.failures.push(info);
                        rocket
                    }
                };

                fairings.last_launch += 1;
            }

            // Note that `rocket.fairings` may now be non-empty since launch
            // fairings could have added more fairings! Move them to the end.
            fairings.append(&mut rocket.fairings);
            rocket.fairings = fairings;
        }

        rocket
    }

    #[inline(always)]
    pub async fn handle_liftoff(&self, rocket: &Rocket<Orbit>) {
        let liftoff_futures = iter!(self.liftoff).map(|f| f.on_liftoff(rocket));
        futures::future::join_all(liftoff_futures).await;
    }

    #[inline(always)]
    pub async fn handle_request(&self, req: &mut Request<'_>, data: &mut Data) {
        for fairing in iter!(self.request) {
            fairing.on_request(req, data).await
        }
    }

    #[inline(always)]
    pub async fn handle_response<'r>(&self, request: &'r Request<'_>, response: &mut Response<'r>) {
        for fairing in iter!(self.response) {
            fairing.on_response(request, response).await;
        }
    }

    pub fn audit(&self) -> Result<(), &[Info]> {
        match self.failures.is_empty() {
            true => Ok(()),
            false => Err(&self.failures)
        }
    }

    pub fn pretty_print(&self) {
        if !self.all_fairings.is_empty() {
            let span = info_span!(target: "rocket::support", "fairings", "{}{}:", Paint::emoji("📡 "), Paint::magenta("Fairings"));
            let _e = span.enter();
            for fairing in &self.all_fairings {
                info!(target: "rocket::support", "{} ({})", Paint::default(fairing.info().name).bold(),
                    Paint::blue(fairing.info().kind).bold());
            }
        }
    }
}

impl std::fmt::Debug for Fairings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn debug_info<'a>(iter: impl Iterator<Item = &'a dyn Fairing>) -> Vec<Info> {
            iter.map(|f| f.info()).collect()
        }

        f.debug_struct("Fairings")
            .field("launch", &debug_info(iter!(self.launch)))
            .field("liftoff", &debug_info(iter!(self.liftoff)))
            .field("request", &debug_info(iter!(self.request)))
            .field("response", &debug_info(iter!(self.response)))
            .finish()
    }
}
