use rocket::async_trait;
use rocket::{Build, Rocket};

use crate::{Config, Error};

/// This trait is implemented on connection pool types that can be used with the
/// [`Database`] derive macro.
///
/// `Pool` determines how the connection pool is initialized from configuration,
/// such as a connection string and optional pool size, along with the returned
/// `Connection` type.
///
/// Implementations of this trait should use `async_trait`.
///
/// ## Example
///
/// ```
/// use rocket::{Build, Rocket};
///
/// #[derive(Debug)]
/// struct Error { /* ... */ }
/// # impl std::fmt::Display for Error {
/// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
/// #         unimplemented!("example")
/// #     }
/// # }
/// # impl std::error::Error for Error { }
///
/// struct Pool { /* ... */ }
/// struct Connection { /* .. */ }
///
/// #[rocket::async_trait]
/// impl rocket_db_pools::Pool for Pool {
///     type Connection = Connection;
///     type InitError = Error;
///     type GetError = Error;
///
///     async fn initialize(db_name: &str, rocket: &Rocket<Build>)
///         -> Result<Self, rocket_db_pools::Error<Self::InitError>>
///     {
///         unimplemented!("example")
///     }
///
///     async fn get(&self) -> Result<Connection, Self::GetError> {
///         unimplemented!("example")
///     }
/// }
/// ```
#[async_trait]
pub trait Pool: Sized + Send + Sync + 'static {
    /// The type returned by get().
    type Connection;

    /// The error type returned by `initialize`.
    type InitError: std::error::Error;

    /// The error type returned by `get`.
    type GetError: std::error::Error;

    /// Constructs a pool from a [Value](rocket::figment::value::Value).
    ///
    /// It is up to each implementor of `Pool` to define its accepted
    /// configuration value(s) via the `Config` associated type.  Most
    /// integrations provided in `rocket_db_pools` use [`Config`], which
    /// accepts a (required) `url` and an (optional) `pool_size`.
    ///
    /// ## Errors
    ///
    /// This method returns an error if the configuration is not compatible, or
    /// if creating a pool failed due to an unavailable database server,
    /// insufficient resources, or another database-specific error.
    async fn initialize(db_name: &str, rocket: &Rocket<Build>)
        -> Result<Self, Error<Self::InitError>>;

    /// Asynchronously gets a connection from the factory or pool.
    ///
    /// ## Errors
    ///
    /// This method returns an error if a connection could not be retrieved,
    /// such as a preconfigured timeout elapsing or when the database server is
    /// unavailable.
    async fn get(&self) -> Result<Self::Connection, Self::GetError>;
}

#[cfg(feature = "deadpool_postgres")]
#[async_trait]
impl Pool for deadpool_postgres::Pool {
    type Connection = deadpool_postgres::Client;
    type InitError = deadpool_postgres::tokio_postgres::Error;
    type GetError = deadpool_postgres::PoolError;

    async fn initialize(db_name: &str, rocket: &Rocket<Build>)
        -> std::result::Result<Self, Error<Self::InitError>>
    {
        let config = Config::from(db_name, rocket)?;
        let manager = deadpool_postgres::Manager::new(
            config.url.parse().map_err(Error::Db)?,
            // TODO: add TLS support in config
            deadpool_postgres::tokio_postgres::NoTls,
        );
        let mut pool_config = deadpool_postgres::PoolConfig::new(config.pool_size as usize);
        pool_config.timeouts.wait = Some(std::time::Duration::from_secs(config.timeout.into()));

        Ok(deadpool_postgres::Pool::from_config(manager, pool_config))
    }

    async fn get(&self) -> Result<Self::Connection, Self::GetError> {
        self.get().await
    }
}

#[cfg(feature = "deadpool_redis")]
#[async_trait]
impl Pool for deadpool_redis::Pool {
    type Connection = deadpool_redis::ConnectionWrapper;
    type InitError = deadpool_redis::redis::RedisError;
    type GetError = deadpool_redis::PoolError;

    async fn initialize(db_name: &str, rocket: &Rocket<Build>)
        -> std::result::Result<Self, Error<Self::InitError>>
    {
        let config = Config::from(db_name, rocket)?;
        let manager = deadpool_redis::Manager::new(config.url).map_err(Error::Db)?;

        let mut pool_config = deadpool_redis::PoolConfig::new(config.pool_size as usize);
        pool_config.timeouts.wait = Some(std::time::Duration::from_secs(config.timeout.into()));

        Ok(deadpool_redis::Pool::from_config(manager, pool_config))
    }

    async fn get(&self) -> Result<Self::Connection, Self::GetError> {
        self.get().await
    }
}

#[cfg(feature = "mongodb")]
#[async_trait]
impl Pool for mongodb::Client {
    type Connection = mongodb::Client;
    type InitError = mongodb::error::Error;
    type GetError = std::convert::Infallible;

    async fn initialize(db_name: &str, rocket: &Rocket<Build>)
        -> std::result::Result<Self, Error<Self::InitError>>
    {
        let config = Config::from(db_name, rocket)?;
        let mut options = mongodb::options::ClientOptions::parse(&config.url)
            .await
            .map_err(Error::Db)?;
        options.max_pool_size = Some(config.pool_size);
        options.wait_queue_timeout = Some(std::time::Duration::from_secs(config.timeout.into()));

        mongodb::Client::with_options(options).map_err(Error::Db)
    }

    async fn get(&self) -> Result<Self::Connection, Self::GetError> {
        Ok(self.clone())
    }
}

#[cfg(feature = "mysql_async")]
#[async_trait]
impl Pool for mysql_async::Pool {
    type Connection = mysql_async::Conn;
    type InitError = mysql_async::Error;
    type GetError = mysql_async::Error;

    async fn initialize(db_name: &str, rocket: &Rocket<Build>)
        -> std::result::Result<Self, Error<Self::InitError>>
    {
        use rocket::figment::{self, error::{Actual, Kind}};

        let config = Config::from(db_name, rocket)?;
        let original_opts = mysql_async::Opts::from_url(&config.url)
            .map_err(|_| figment::Error::from(Kind::InvalidValue(
                Actual::Str(config.url.to_string()),
                "mysql connection string".to_string()
            )))?;

        let new_pool_opts = original_opts.pool_opts()
            .clone()
            .with_constraints(
                mysql_async::PoolConstraints::new(0, config.pool_size as usize)
                    .expect("usize can't be < 0")
            );

        // TODO: timeout

        let opts = mysql_async::OptsBuilder::from_opts(original_opts)
            .pool_opts(new_pool_opts);

        Ok(mysql_async::Pool::new(opts))
    }

    async fn get(&self) -> std::result::Result<Self::Connection, Self::GetError> {
        self.get_conn().await
    }
}

#[cfg(feature = "sqlx_mysql")]
#[async_trait]
impl Pool for sqlx::MySqlPool {
    type Connection = sqlx::pool::PoolConnection<sqlx::MySql>;
    type InitError = sqlx::Error;
    type GetError = sqlx::Error;

    async fn initialize(db_name: &str, rocket: &Rocket<Build>)
        -> std::result::Result<Self, Error<Self::InitError>>
    {
        use sqlx::ConnectOptions;

        let config = Config::from(db_name, rocket)?;
        let mut opts = config.url.parse::<sqlx::mysql::MySqlConnectOptions>()
            .map_err(Error::Db)?;
        opts.disable_statement_logging();
        sqlx::pool::PoolOptions::new()
            .max_connections(config.pool_size)
            .connect_timeout(std::time::Duration::from_secs(config.timeout.into()))
            .connect_with(opts)
            .await
            .map_err(Error::Db)
    }

    async fn get(&self) -> std::result::Result<Self::Connection, Self::GetError> {
        self.acquire().await
    }
}

#[cfg(feature = "sqlx_postgres")]
#[async_trait]
impl Pool for sqlx::PgPool {
    type Connection = sqlx::pool::PoolConnection<sqlx::Postgres>;
    type InitError = sqlx::Error;
    type GetError = sqlx::Error;

    async fn initialize(db_name: &str, rocket: &Rocket<Build>)
        -> std::result::Result<Self, Error<Self::InitError>>
    {
        use sqlx::ConnectOptions;

        let config = Config::from(db_name, rocket)?;
        let mut opts = config.url.parse::<sqlx::postgres::PgConnectOptions>()
            .map_err(Error::Db)?;
        opts.disable_statement_logging();
        sqlx::pool::PoolOptions::new()
            .max_connections(config.pool_size)
            .connect_timeout(std::time::Duration::from_secs(config.timeout.into()))
            .connect_with(opts)
            .await
            .map_err(Error::Db)
    }

    async fn get(&self) -> std::result::Result<Self::Connection, Self::GetError> {
        self.acquire().await
    }
}

#[cfg(feature = "sqlx_sqlite")]
#[async_trait]
impl Pool for sqlx::SqlitePool {
    type Connection = sqlx::pool::PoolConnection<sqlx::Sqlite>;
    type InitError = sqlx::Error;
    type GetError = sqlx::Error;

    async fn initialize(db_name: &str, rocket: &Rocket<Build>)
        -> std::result::Result<Self, Error<Self::InitError>>
    {
        use sqlx::ConnectOptions;

        let config = Config::from(db_name, rocket)?;
        let mut opts = config.url.parse::<sqlx::sqlite::SqliteConnectOptions>()
            .map_err(Error::Db)?
            .create_if_missing(true);
        opts.disable_statement_logging();

        dbg!(sqlx::pool::PoolOptions::new()
            .max_connections(config.pool_size)
            .connect_timeout(std::time::Duration::from_secs(config.timeout.into())))
            .connect_with(opts)
            .await
            .map_err(Error::Db)
    }

    async fn get(&self) -> std::result::Result<Self::Connection, Self::GetError> {
        self.acquire().await
    }
}
