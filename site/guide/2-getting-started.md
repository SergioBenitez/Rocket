# Getting Started

Let's create and run our first Rocket application. We'll ensure we have a
compatible Rust toolchain installed, create a new Cargo project that depends on
Rocket, and then run the application.

## Installing Rust

Rocket makes use of the latest Rust features. Because of this, we'll need a
recent release of Rust to run Rocket applications. If you already have a working
installation of the latest Rust compiler, feel free to skip to the next section.

To install the latest version of Rust, we recommend using `rustup`. Install
`rustup` by following the instructions on [its website](https://rustup.rs/).
Once `rustup` is installed, ensure the latest toolchain is installled by running
the command:

```sh
rustup default stable
```

! note: You may prefer to develop using the _nightly_ channel.

  The nightly Rust toolchain enables certain improved developer experiences,
  such as better compile-time diagnostics, when developing with Rocket. You may
  choose to develop on the nightly channel to take advantage of these improved
  experiences. Note that all Rocket features are available across all Rust
  channels.

  To set the nightly toolchain as your default, run `rustup default nightly`.

## Hello, world!

Let's write our first Rocket application! Start by creating a new binary-based
Cargo project and changing into the new directory:

```sh
cargo new hello-rocket --bin
cd hello-rocket
```

Now, add Rocket as a dependency in your `Cargo.toml`:

```toml
[dependencies]
rocket = "0.5.0-rc.1"
```

! warning: Development versions must be _git_ dependencies.

  Development versions, tagged with `-dev`, are not published. To depend on a
  development version of Rocket, you'll need to point `Cargo.toml` to a Rocket
  git repository. For example, with `######` replaced with a git commit hash:

  `
  [dependencies]
  `
  `
  rocket = { git = "https://github.com/SergioBenitez/Rocket", rev = "######" }
  `

Modify `src/main.rs` so that it contains the code for the Rocket `Hello, world!`
program, reproduced below:

```rust
#[macro_use] extern crate rocket;

#[get("/")]
fn index() -> &'static str {
    "Hello, world!"
}

#[launch]
fn rocket() -> _ {
    rocket::build().mount("/", routes![index])
}
```

We won't explain exactly what the program does now; we leave that for the rest
of the guide. In short, it creates an `index` route, _mounts_ the route at the
`/` path, and launches the application. Compile and run the program with `cargo
run`. You should see the following:

```sh
> cargo run
🔧 Configured for debug.
   >> address: 127.0.0.1
   >> port: 8000
   >> workers: [..]
   >> keep-alive: 5s
   >> limits: [..]
   >> tls: disabled
   >> temp dir: /tmp
   >> log level: normal
   >> cli colors: true
🛰  Routes:
   >> (index) GET /
🚀 Rocket has launched from http://127.0.0.1:8000
```

Visit `http://localhost:8000` to see your first Rocket application in action!

! tip: Don't like colors or emoji?

  You can disable colors and emoji by setting the `ROCKET_CLI_COLORS`
  environment variable to `0` or `false` when running a Rocket binary:
  `ROCKET_CLI_COLORS=false cargo run`.
