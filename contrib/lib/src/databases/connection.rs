use std::marker::PhantomData;
use std::sync::Arc;

use rocket::{Rocket, Phase};
use rocket::fairing::{AdHoc, Fairing};
use rocket::request::{Request, Outcome, FromRequest};
use rocket::trace::error;
use rocket::outcome::IntoOutcome;
use rocket::http::Status;

use rocket::tokio::sync::{OwnedSemaphorePermit, Semaphore, Mutex};
use rocket::tokio::time::timeout;

use crate::databases::{Config, Poolable, Error};

/// Unstable internal details of generated code for the #[database] attribute.
///
/// This type is implemented here instead of in generated code to ensure all
/// types are properly checked.
#[doc(hidden)]
pub struct ConnectionPool<K, C: Poolable> {
    config: Config,
    // This is an 'Option' so that we can drop the pool in a 'spawn_blocking'.
    pool: Option<r2d2::Pool<C::Manager>>,
    semaphore: Arc<Semaphore>,
    _marker: PhantomData<fn() -> K>,
}

impl<K, C: Poolable> Clone for ConnectionPool<K, C> {
    fn clone(&self) -> Self {
        ConnectionPool {
            config: self.config.clone(),
            pool: self.pool.clone(),
            semaphore: self.semaphore.clone(),
            _marker: PhantomData
        }
    }
}

/// Unstable internal details of generated code for the #[database] attribute.
///
/// This type is implemented here instead of in generated code to ensure all
/// types are properly checked.
#[doc(hidden)]
pub struct Connection<K, C: Poolable> {
    connection: Arc<Mutex<Option<r2d2::PooledConnection<C::Manager>>>>,
    permit: Option<OwnedSemaphorePermit>,
    _marker: PhantomData<fn() -> K>,
}

// A wrapper around spawn_blocking that propagates panics to the calling code.
async fn run_blocking<F, R>(job: F) -> R
    where F: FnOnce() -> R + Send + 'static, R: Send + 'static,
{
    match tokio::task::spawn_blocking(job).await {
        Ok(ret) => ret,
        Err(e) => match e.try_into_panic() {
            Ok(panic) => std::panic::resume_unwind(panic),
            Err(_) => unreachable!("spawn_blocking tasks are never cancelled"),
        }
    }
}

macro_rules! dberr {
    ($msg:literal, $db_name:expr, $efmt:literal, $error:expr, $rocket:expr) => ({
        error!(concat!("database ", $msg, " error for pool named `{}`"), $db_name);
        error!($efmt, $error);
        return Err($rocket);
    });
}

impl<K: 'static, C: Poolable> ConnectionPool<K, C> {
    pub fn fairing(fairing_name: &'static str, db: &'static str) -> impl Fairing {
        AdHoc::try_on_ignite(fairing_name, move |rocket| async move {
            let config = match Config::from(db, &rocket) {
                Ok(config) => config,
                Err(e) => dberr!("config", db, "{}", e, rocket),
            };

            let pool_size = config.pool_size;
            match C::pool(db, &rocket) {
                Ok(pool) => Ok(rocket.manage(ConnectionPool::<K, C> {
                    config,
                    pool: Some(pool),
                    semaphore: Arc::new(Semaphore::new(pool_size as usize)),
                    _marker: PhantomData,
                })),
                Err(Error::Config(e)) => dberr!("config", db, "{}", e, rocket),
                Err(Error::Pool(e)) => dberr!("pool init", db, "{}", e, rocket),
                Err(Error::Custom(e)) => dberr!("pool manager", db, "{:?}", e, rocket),
            }
        })
    }

    async fn get(&self) -> Result<Connection<K, C>, ()> {
        let duration = std::time::Duration::from_secs(self.config.timeout as u64);
        let permit = match timeout(duration, self.semaphore.clone().acquire_owned()).await {
            Ok(p) => p.expect("internal invariant broken: semaphore should not be closed"),
            Err(_) => {
                error!("database connection retrieval timed out");
                return Err(());
            }
        };

        let pool = self.pool.as_ref().cloned()
            .expect("internal invariant broken: self.pool is Some");

        match run_blocking(move || pool.get_timeout(duration)).await {
            Ok(c) => Ok(Connection {
                connection: Arc::new(Mutex::new(Some(c))),
                permit: Some(permit),
                _marker: PhantomData,
            }),
            Err(e) => {
                error!("failed to get a database connection: {}", e);
                Err(())
            }
        }
    }

    #[inline]
    pub async fn get_one<P: Phase>(rocket: &Rocket<P>) -> Option<Connection<K, C>> {
        match rocket.state::<Self>() {
            Some(pool) => match pool.get().await.ok() {
                Some(conn) => Some(conn),
                None => {
                    error!("no connections available for `{}`", std::any::type_name::<K>());
                    None
                }
            },
            None => {
                error!("missing database fairing for `{}`", std::any::type_name::<K>());
                None
            }
        }
    }

    #[inline]
    pub async fn get_pool<P: Phase>(rocket: &Rocket<P>) -> Option<Self> {
        rocket.state::<Self>().map(|pool| pool.clone())
    }
}

impl<K: 'static, C: Poolable> Connection<K, C> {
    #[inline]
    pub async fn run<F, R>(&self, f: F) -> R
        where F: FnOnce(&mut C) -> R + Send + 'static,
              R: Send + 'static,
    {
        let mut connection = self.connection.clone().lock_owned().await;
        run_blocking(move || {
            let conn = connection.as_mut()
                .expect("internal invariant broken: self.connection is Some");
            f(conn)
        }).await
    }
}

impl<K, C: Poolable> Drop for Connection<K, C> {
    fn drop(&mut self) {
        let connection = self.connection.clone();
        let permit = self.permit.take();
        tokio::spawn(async move {
            let mut connection = connection.lock_owned().await;
            tokio::task::spawn_blocking(move || {
                if let Some(conn) = connection.take() {
                    drop(conn);
                }

                // Explicitly dropping the permit here so that it's only
                // released after the connection is.
                drop(permit);
            })
        });
    }
}

impl<K, C: Poolable> Drop for ConnectionPool<K, C> {
    fn drop(&mut self) {
        let pool = self.pool.take();
        tokio::task::spawn_blocking(move || drop(pool));
    }
}

#[rocket::async_trait]
impl<'r, K: 'static, C: Poolable> FromRequest<'r> for Connection<K, C> {
    type Error = ();

    #[inline]
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, ()> {
        match request.rocket().state::<ConnectionPool<K, C>>() {
            Some(c) => c.get().await.into_outcome(Status::ServiceUnavailable),
            None => {
                error!("Missing database fairing for `{}`", std::any::type_name::<K>());
                Outcome::Failure((Status::InternalServerError, ()))
            }
        }
    }
}
