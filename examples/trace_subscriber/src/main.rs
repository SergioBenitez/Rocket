#[macro_use] extern crate rocket;

use rocket::trace::{debug, info, instrument};

#[get("/hello/<name>/<age>")]
fn hello(name: &str, age: u8) -> String {
    info!("saying hello...");
    greet(name, Some(age))
}

#[get("/hello/<name>")]
fn hi(name: &str) -> String {
    info!(?name, "saying hi to");
    greet(name, None)
}

#[instrument(level = "info")]
fn greet(name: &str, age: Option<u8>) -> String {
    debug!(?age);
    if let Some(age) = age {
        debug!("found an age, saying hello");
        format!("Hello, {} year old named {}!", age, name)
    } else {
        debug!("no age, just saying hi...");
        name.to_string()
    }
}

#[launch]
fn rocket() -> _ {
    use rocket::trace::prelude::*;

    let figment = rocket::Config::figment();
    let config = rocket::Config::from(&figment);

    // Use Rocket's default log filter...
    let filter = rocket::trace::filter_layer(config.log_level)
        // ...but enable the debug level for the app crate.
        .add_directive("trace_subscriber=debug".parse().unwrap());

    rocket::trace::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();

    rocket::custom(figment).mount("/", routes![hello, hi])
}
