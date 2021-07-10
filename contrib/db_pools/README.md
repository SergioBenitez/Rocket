# `db_pools` [![ci.svg]][ci] [![crates.io]][crate] [![docs.svg]][crate docs]

[crates.io]: https://img.shields.io/crates/v/rocket_db_pools.svg
[crate]: https://crates.io/crates/rocket_db_pools
[docs.svg]: https://img.shields.io/badge/web-master-red.svg?style=flat&label=docs&colorB=d33847
[crate docs]: https://api.rocket.rs/master/rocket_db_pools
[ci.svg]: https://github.com/SergioBenitez/Rocket/workflows/CI/badge.svg
[ci]: https://github.com/SergioBenitez/Rocket/actions

This crate provides traits, utilities, and a procedural macro for configuring
and accessing database connection pools in Rocket.

## Usage

First, enable the feature corresponding to your database type:

```toml
[dependencies.rocket_db_pools]
version = "0.1.0-dev"
features = ["sqlx_sqlite"]
```

A full list of supported databases and their associated feature names is
available in the [crate docs]. In whichever configuration source you choose,
configure a `databases` dictionary with a key for each database, here
`sqlite_logs` in a TOML source:

```toml
[default.databases]
sqlite_logs = { url = "/path/to/database.sqlite" }
```

In your application's source code:

```rust
#[macro_use] extern crate rocket;
use rocket::serde::json::Json;

use rocket_db_pools::{Database, sqlx};

#[derive(Database)]
#[database("sqlite_logs")]
struct LogsDb(sqlx::SqlitePool);

type LogsDbConn = <LogsDb as Database>::Connection;

#[get("/logs/<id>")]
async fn get_logs(mut db: LogsDbConn, id: usize) -> Result<Json<Vec<String>>> {
    let logs = sqlx::query!("SELECT text FROM logs;").execute(&mut *db).await?;

    Ok(Json(logs))
}

#[launch]
fn rocket() -> _ {
    rocket::build().attach(LogsDb::fairing())
}
```

See the [crate docs] for full details.
