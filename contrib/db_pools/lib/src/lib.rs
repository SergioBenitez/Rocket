//! Traits, utilities, and a macro for easy database connection pooling.
//!
//! # Overview
//!
//! This crate provides traits, utilities, and a procedural macro for
//! configuring and accessing database connection pools in Rocket. A _database
//! connection pool_ is a data structure that maintains active database
//! connections for later use in the application.
//!
//! Databases are individually configured through Rocket's regular configuration
//! mechanisms. Connecting a Rocket application to a database using this library
//! occurs in three simple steps:
//!
//!   1. Configure your databases in `Rocket.toml`.
//!      (see [Configuration](#configuration))
//!   2. Associate a Database type and fairing with each database.
//!      (see [Guard Types](#guard-types))
//!   3. Use the request guard to retrieve a connection in a handler.
//!      (see [Handlers](#handlers))
//!
//! For a list of supported databases, see [Provided Databases](#provided). This
//! support can be easily extended by implementing the [`Pool`] trait. See
//! [Extending](#extending) for more.
//!
//! ## Example
//!
//! Before using this library, the feature corresponding to your database type
//! in `rocket_db_pools` must be enabled:
//!
//! ```toml
//! [dependencies.rocket_db_pools]
//! version = "0.1.0-dev"
//! features = ["sqlx_sqlite"]
//! ```
//!
//! See [Provided](#provided) for a list of supported database and their
//! associated feature name.
//!
//! In whichever configuration source you choose, configure a `databases`
//! dictionary with an internal dictionary for each database, here `sqlite_logs`
//! in a TOML source:
//!
//! ```toml
//! [default.databases]
//! sqlite_logs = { url = "/path/to/database.sqlite" }
//! ```
//!
//! In your application's source code, one-time:
//!
//! ```rust
//! # #[macro_use] extern crate rocket;
//! # #[cfg(feature = "sqlx_sqlite")]
//! # mod test {
//! use rocket_db_pools::{Database, Connection, sqlx};
//!
//! #[derive(Database)]
//! #[database("sqlite_logs")]
//! struct LogsDb(sqlx::SqlitePool);
//!
//! type LogsDbConn = Connection<LogsDb>;
//!
//! #[launch]
//! fn rocket() -> _ {
//!     rocket::build().attach(LogsDb::fairing())
//! }
//! # } fn main() {}
//! ```
//!
//! These steps can be repeated as many times as necessary to configure
//! multiple databases.
//!
//! Whenever a connection to the database is needed:
//!
//! ```rust
//! # #[macro_use] extern crate rocket;
//! # #[macro_use] extern crate rocket_db_pools;
//! #
//! # #[cfg(feature = "sqlx_sqlite")]
//! # mod test {
//! # use rocket::serde::json::Json;
//! # use rocket_db_pools::{Database, Connection, sqlx};
//! #
//! # #[derive(Database)]
//! # #[database("sqlite_logs")]
//! # struct LogsDb(sqlx::SqlitePool);
//! # type LogsDbConn = Connection<LogsDb>;
//! #
//! # type Result<T> = std::result::Result<T, ()>;
//! #
//! #[get("/logs/<id>")]
//! async fn get_logs(conn: LogsDbConn, id: usize) -> Result<Json<Vec<String>>> {
//! # /*
//!     let logs = sqlx::query!().await?;
//!     Ok(Json(logs))
//! # */
//! # Ok(Json(vec![]))
//! }
//! # } fn main() {}
//! ```
//!
//! # Usage
//!
//! ## Configuration
//!
//! Databases can be configured as any other values. Using the default
//! configuration provider, either via `Rocket.toml` or environment variables.
//! You can also use a custom provider.
//!
//! ### `Rocket.toml`
//!
//! To configure a database via `Rocket.toml`, add a table for each database
//! to the `databases` table where the key is a name of your choice. The table
//! should have a `url` key and, optionally, a `pool_size` key. This looks as
//! follows:
//!
//! ```toml
//! # Option 1:
//! [global.databases]
//! sqlite_db = { url = "db.sqlite" }
//!
//! # Option 2:
//! [global.databases.my_db]
//! url = "postgres://root:root@localhost/my_db"
//!
//! # With a `pool_size` key:
//! [global.databases]
//! sqlite_db = { url = "db.sqlite", pool_size = 20 }
//! ```
//!
//! Most databases use the default [`Config`] type, for which one key is required:
//!
//!   * `url` - the URl to the database
//!
//! And one optional key is accepted:
//!
//!   * `pool_size` - the size of the pool, i.e., the number of connections to
//!     pool (defaults to the configured number of workers * 4)
//!       TODO: currently ignored by most `Pool` implementations.
//!
//! Different options may be required or supported by other adapters, according
//! to the type specified by [`Pool::Config`].
//!
//! ### Procedurally
//!
//! Databases can also be configured procedurally via `rocket::custom()`.
//! The example below does just this:
//!
//! ```rust
//! # #[cfg(feature = "sqlx_sqlite")] {
//! # use rocket::launch;
//! use rocket::figment::{value::{Map, Value}, util::map};
//!
//! #[launch]
//! fn rocket() -> _ {
//!     let db: Map<_, Value> = map! {
//!         "url" => "db.sqlite".into(),
//!         "pool_size" => 10.into()
//!     };
//!
//!     let figment = rocket::Config::figment()
//!         .merge(("databases", map!["my_db" => db]));
//!
//!     rocket::custom(figment)
//! }
//! # rocket();
//! # }
//! ```
//!
//! ### Environment Variables
//!
//! Lastly, databases can be configured via environment variables by specifying
//! the `databases` table as detailed in the [Environment Variables
//! configuration
//! guide](https://rocket.rs/master/guide/configuration/#environment-variables):
//!
//! ```bash
//! ROCKET_DATABASES='{my_db={url="db.sqlite"}}'
//! ```
//!
//! Multiple databases can be specified in the `ROCKET_DATABASES` environment variable
//! as well by comma separating them:
//!
//! ```bash
//! ROCKET_DATABASES='{my_db={url="db.sqlite"},my_pg_db={url="postgres://root:root@localhost/my_pg_db"}}'
//! ```
//!
//! ## Database Types
//!
//! Once a database has been configured, the `#[derive(Database)]` macro can be
//! used to tie a type in your application to a configured database. The derive
//! accepts a single attribute, `#[database("name")]` that indicates the
//! name of the database. This corresponds to the database name set as the
//! database's configuration key.
//!
//! The [`Database`] trait provides a method, `fairing()`, which places an
//! instance of the decorated type in managed state; thus, the database pool can
//! be accessed with a `&State<DbType>` request guard.
//!
//! The [`Connection`] type also implements [`FromRequest`], allowing it to be
//! used as a request guard. This implementation retrieves a connection from the
//! database pool or fails with a `Status::ServiceUnavailable` if connecting to
//! the database fails or times out.
//!
//! The derive can only be applied to unit-like structs with one type. The
//! internal type of the structure must implement [`Pool`].
//!
//! ```rust
//! # #[macro_use] extern crate rocket_db_pools;
//! # #[cfg(feature = "sqlx_sqlite")]
//! # mod test {
//! use rocket_db_pools::{Database, sqlx};
//!
//! #[derive(Database)]
//! #[database("my_db")]
//! struct MyDatabase(sqlx::SqlitePool);
//! # }
//! ```
//!
//! Other databases can be used by specifying their respective [`Pool`] type:
//!
//! ```rust
//! # #[macro_use] extern crate rocket_db_pools;
//! # #[cfg(feature = "deadpool_postgres")]
//! # mod test {
//! use rocket_db_pools::{Database, deadpool_postgres};
//!
//! #[derive(Database)]
//! #[database("my_pg_db")]
//! struct MyPgDatabase(deadpool_postgres::Pool);
//! # }
//! ```
//!
//! The fairing returned from the `fairing()` method _must_ be attached for the
//! request guards to succeed. Putting the pieces together, a use of
//! `#[derive(Database)]` looks as follows:
//!
//! ```rust
//! # #[macro_use] extern crate rocket;
//! # #[macro_use] extern crate rocket_db_pools;
//! #
//! # #[cfg(feature = "sqlx_sqlite")] {
//! # use rocket::figment::{value::{Map, Value}, util::map};
//! use rocket_db_pools::{Database, sqlx};
//!
//! #[derive(Database)]
//! #[database("my_db")]
//! struct MyDatabase(sqlx::SqlitePool);
//!
//! #[launch]
//! fn rocket() -> _ {
//! #   let db: Map<_, Value> = map![
//! #        "url" => "db.sqlite".into(), "pool_size" => 10.into()
//! #   ];
//! #   let figment = rocket::Config::figment().merge(("databases", map!["my_db" => db]));
//!     rocket::custom(figment).attach(MyDatabase::fairing())
//! }
//! # }
//! ```
//!
//! ## Handlers
//!
//! Finally, access your type via `State` in a handler to access
//! the database connection pool:
//!
//! ```rust
//! # #[macro_use] extern crate rocket;
//! # #[macro_use] extern crate rocket_db_pools;
//! #
//! # #[cfg(feature = "sqlx_sqlite")]
//! # mod test {
//! # use rocket_db_pools::{Database, Connection, sqlx};
//! use rocket::State;
//!
//! #[derive(Database)]
//! #[database("my_db")]
//! struct MyDatabase(sqlx::SqlitePool);
//!
//! #[get("/")]
//! fn my_handler(conn: &State<MyDatabase>) {
//!     // ...
//! }
//! # }
//! ```
//!
//! Alternatively, access a single connection directly via the `Connection`
//! request guard:
//!
//! ```rust
//! # #[macro_use] extern crate rocket;
//! # #[macro_use] extern crate rocket_db_pools;
//! #
//! # #[cfg(feature = "sqlx_sqlite")]
//! # mod test {
//! # use rocket_db_pools::{Database, Connection, sqlx};
//! # type Data = ();
//! #[derive(Database)]
//! #[database("my_db")]
//! struct MyDatabase(sqlx::SqlitePool);
//!
//! type MyConnection = Connection<MyDatabase>;
//!
//! async fn load_from_db(conn: &mut sqlx::SqliteConnection) -> Data {
//!     // Do something with connection, return some data.
//!     # ()
//! }
//!
//! #[get("/")]
//! async fn my_handler(mut conn: MyConnection) -> Data {
//!     load_from_db(&mut conn).await
//! }
//! # }
//! ```
//!
//! # Database Support
//!
//! Built-in support is provided for many popular databases and drivers. Support
//! can be easily extended by [`Pool`] implementations.
//!
//! ## Provided
//!
//! The list below includes all presently supported database adapters and their
//! corresponding [`Pool`] type.
//!
// Note: Keep this table in sync with site/guite/6-state.md
//! | Kind     | Driver                | Version   | `Pool` Type                    | Feature                |
//! |----------|-----------------------|-----------|--------------------------------|------------------------|
//! | MySQL    | [sqlx]                | `0.5`     | [`sqlx::MySqlPool`]            | `sqlx_mysql`           |
//! | Postgres | [sqlx]                | `0.5`     | [`sqlx::PgPool`]               | `sqlx_postgres`        |
//! | Sqlite   | [sqlx]                | `0.5`     | [`sqlx::SqlitePool`]           | `sqlx_sqlite`          |
//! | Mongodb  | [mongodb]             | `2.0.0-beta` | [`mongodb::Client`]         | `mongodb`              |
//! | MySQL    | [mysql_async]         | `0.27`    | [`mysql_async::Pool`]          | `mysql_async`          |
//! | Postgres | [deadpool-postgres]   | `0.8`     | [`deadpool_postgres::Pool`]    | `deadpool_postgres`    |
//! | Redis    | [deadpool-redis]      | `0.8`     | [`deadpool_redis::Pool`]       | `deadpool_redis`       |
//!
//! [sqlx]: https://docs.rs/sqlx/0.5/sqlx/
//! [deadpool-postgres]: https://docs.rs/deadpool-postgres/0.8/deadpool_postgres/
//! [deadpool-redis]: https://docs.rs/deadpool-redis/0.8/deadpool_redis/
//! [mongodb]: https://docs.rs/mongodb/2.0.0-beta/mongodb/index.html
//! [mysql_async]: https://docs.rs/mysql_async/0.27/mysql_async/
//!
//! The above table lists all the supported database adapters in this library.
//! In order to use particular `Pool` type that's included in this library,
//! you must first enable the feature listed in the "Feature" column. The
//! interior type of your decorated database type should match the type in the
//! "`Pool` Type" column.
//!
//! ## Extending
//!
//! Extending Rocket's support to your own custom database adapter is as easy as
//! implementing the [`Pool`] trait. See the documentation for [`Pool`]
//! for more details on how to implement it.
//!
//! [`FromRequest`]: rocket::request::FromRequest
//! [request guards]: rocket::request::FromRequest
//! [`Database`]: crate::Database
//! [`Pool`]: crate::Pool

#![doc(html_root_url = "https://api.rocket.rs/master/rocket_db_pools")]
#![doc(html_favicon_url = "https://rocket.rs/images/favicon.ico")]
#![doc(html_logo_url = "https://rocket.rs/images/logo-boxed.png")]

#[doc(hidden)]
#[macro_use]
pub extern crate rocket;

#[cfg(feature = "deadpool_postgres")] pub use deadpool_postgres;
#[cfg(feature = "deadpool_redis")] pub use deadpool_redis;
#[cfg(feature = "mysql_async")] pub use mysql_async;
#[cfg(feature = "mongodb")] pub use mongodb;
#[cfg(feature = "sqlx")] pub use sqlx;

mod config;
mod database;
mod error;
mod pool;

pub use self::config::Config;
pub use self::database::{Connection, Database, Fairing};
pub use self::error::Error;
pub use self::pool::Pool;

pub use rocket_db_pools_codegen::*;
