use rocket::fairing::{Info, Kind};
use rocket::futures::future::BoxFuture;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};
use rocket::yansi::Paint;
use rocket::{Build, Ignite, Rocket, Sentinel};

use crate::{Error, Pool};

/// Trait implemented to define a database connection pool.
pub trait Database: Sized + Send + Sync + 'static {
    /// The name of this connection pool in the configuration.
    const NAME: &'static str;

    /// The underlying connection type returned by this pool.
    /// Must implement [`Pool`].
    type Pool: Pool;

    /// Returns a fairing that attaches this connection pool to the server.
    fn fairing() -> Fairing<Self>;

    /// Direct shared access to the underlying database pool
    fn pool(&self) -> &Self::Pool;

    /// get().await returns a connection from the pool (or an error)
    fn get(&self) -> BoxFuture<'_, Result<Connection<Self>, <Self::Pool as Pool>::GetError>> {
        Box::pin(async move { self.pool().get().await.map(Connection)} )
    }
}

/// A connection. The underlying connection type is determined by `D`, which
/// must implement [`Database`].
pub struct Connection<D: Database>(<D::Pool as Pool>::Connection);

impl<D: Database> std::ops::Deref for Connection<D> {
    type Target = <D::Pool as Pool>::Connection;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<D: Database> std::ops::DerefMut for Connection<D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[rocket::async_trait]
impl<'r, D: Database> FromRequest<'r> for Connection<D> {
    type Error = Error<<D::Pool as Pool>::GetError>;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let db: &D = match req.rocket().state() {
            Some(p) => p,
            _ => {
                let dbtype = Paint::default(std::any::type_name::<D>()).bold();
                let fairing = Paint::default(format!("{}::fairing()", dbtype)).wrap().bold();
                error!("requesting `{}` DB connection without attaching `{}`.", dbtype, fairing);
                info_!("Attach `{}` to use database connection pooling.", fairing);
                return Outcome::Failure((Status::InternalServerError, Error::UnattachedFairing));
            }
        };

        match db.pool().get().await {
            Ok(conn) => Outcome::Success(Connection(conn)),
            Err(e) => Outcome::Failure((Status::ServiceUnavailable, Error::Db(e))),
        }
    }
}

impl<D: Database> Sentinel for Connection<D> {
    fn abort(rocket: &Rocket<Ignite>) -> bool {
        if rocket.state::<D>().is_none() {
            let dbtype = Paint::default(std::any::type_name::<D>()).bold();
            let fairing = Paint::default(format!("{}::fairing()", dbtype)).wrap().bold();
            error!("requesting `{}` DB connection without attaching `{}`.", dbtype, fairing);
            info_!("Attach `{}` to use database connection pooling.", fairing);
            return true;
        }

        false
    }
}

/// The database fairing for pool types created with the `pool!` macro.
pub struct Fairing<D: Database>(&'static str, std::marker::PhantomData<fn(D::Pool)>);

impl<D: Database + From<D::Pool>> Fairing<D> {
    /// Create a new database fairing with the given constructor.  This
    /// constructor will be called to create an instance of `D` after the pool
    /// is initialized and before it is placed into managed state.
    pub fn new(fairing_name: &'static str) -> Self {
        Self(fairing_name, std::marker::PhantomData)
    }
}

#[rocket::async_trait]
impl<D: Database + From<D::Pool>> rocket::fairing::Fairing for Fairing<D> {
    fn info(&self) -> Info {
        Info {
            name: self.0,
            kind: Kind::Ignite,
        }
    }

    async fn on_ignite(&self, rocket: Rocket<Build>) -> Result<Rocket<Build>, Rocket<Build>> {
        let pool = match <D::Pool>::initialize(D::NAME, &rocket).await {
            Ok(p) => p,
            Err(e) => {
                error!("error initializing database connection pool: {}", e);
                return Err(rocket);
            }
        };

        let db: D = pool.into();

        Ok(rocket.manage(db))
    }
}
