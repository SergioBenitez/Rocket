use rocket::figment::{self, Figment, providers::Serialized};
use rocket::serde::{Deserialize, Serialize};
use rocket::{Build, Rocket};

/// A base `Config` for any `Pool` type.
///
/// For the following configuration:
///
/// ```toml
/// [global.databases.my_database]
/// url = "postgres://root:root@localhost/my_database"
/// pool_size = 10
/// ```
///
/// ...the following struct would be passed to [`Pool::initialize()`]:
///
/// ```rust
/// # use rocket_db_pools::Config;
/// Config {
///     url: "postgres://root:root@localhost/my_database".into(),
///     pool_size: 10,
///     timeout: 5,
/// };
/// ```
///
/// If you want to implement your own custom database adapter and need some more
/// configuration options, you may need to define a custom `Config` struct.
///
/// [`Pool::initialize()`]: crate::Pool::initialize
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(crate = "rocket::serde")]
pub struct Config {
    /// Connection URL specified in the Rocket configuration.
    pub url: String,
    /// Initial pool size. Defaults to the number of Rocket workers * 4.
    pub pool_size: u32,
    /// How long to wait, in seconds, for a new connection before timing out.
    /// Defaults to `5`.
    // FIXME: Use `time`.
    pub timeout: u8,
}

impl Config {
    pub fn from(db_name: &str, rocket: &Rocket<Build>) -> Result<Self, figment::Error> {
        Self::figment(db_name, rocket).extract::<Self>()
    }

    pub fn figment(db_name: &str, rocket: &Rocket<Build>) -> Figment {
        let db_key = format!("databases.{}", db_name);
        let default_pool_size = rocket.figment()
            .extract_inner::<u32>(rocket::Config::WORKERS)
            .map(|workers| workers * 4)
            .ok();

        let figment = Figment::from(rocket.figment())
            .focus(&db_key)
            .join(Serialized::default("timeout", 5));

        match default_pool_size {
            Some(pool_size) => figment.join(Serialized::default("pool_size", pool_size)),
            None => figment
        }
    }
}
