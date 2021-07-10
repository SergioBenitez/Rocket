# Rocket Examples

This directory contains projects showcasing Rocket's features.

## Applications

  * **[`pastebin`](./pastebin)**

    A simple, API-only pastebin application, similar to https://paste.rs. Stores
    pastes locally on the file system. Implements a custom parameter guard,
    `PasteId`, to parse and validate paste identifiers.

  * **[`todo`](./todo)**

    A todo app with a web UI to add, delete, and mark/unmark items. Uses a
    SQLite database driven by diesel. Runs migrations automatically at start-up.
    Uses tera to render templates.

  * **[`chat`](./chat)**

    A real-time, multi-room chat application using Server-Sent Events (SSE) and
    JavaScript's `EventSource`. Supports automatic reconnection with exponential
    backoff and live connection status.

## Feature Examples

  * **[`config`](./config)** - Illustrates how to extract values from a Rocket
    `Figment`, how to store and retrieve an application specific configuration
    in managed state using `AdHoc::config()`, and how to set configuration
    values in `Rocket.toml`.

  * **[`cookies`](./cookies)** - Uses cookies to create a client-side message
    box. Uses private cookies for a session-based authentication.

  * **[`databases`](./databases)** - Implements a CRUD-like "blog" JSON API
    backed by a SQLite database driven by each of `sqlx`, `diesel`, and
    `rusqlite`. Runs migrations automatically for the former two drivers. Uses
    `contrib` database support for all drivers (`rocket_db_pools` for the first;
    `rocket_sync_db_pools` for the other latter two).

  * **[`error-handling`](./error-handling)** - Exhibits the use of scoped
    catchers; contains commented out lines that will cause a launch-time error
    with code to custom-display the error.

  * **[`fairings`](./fairings)** - Exemplifies creating a custom `Counter`
    fairing and using `AdHoc` fairings.

  * **[`forms`](./forms)** - Showcases all of Rocket's form support features
    including multipart file uploads, ad-hoc validations, field renaming, and
    use of form context for staged forms.

  * **[`hello`](./hello)** - Basic example of Rocket's core features: route
    declaration with path and query parameters, both simple and compound,
    mounting, launching, testing, and returning simple responses. Also showcases
    using UTF-8 in route declarations and responses.

  * **[`manual-routing`](./manual-routing)** - An example eschewing Rocket's
    codegen in favor of manual routing. This should be seen as last-ditch
    effort, much like `unsafe` in Rust, as manual routing _also_ eschews many of
    Rocket's automatic web security guarantees.

  * **[`responders`](./responders)** - Illustrates the use of many of Rocket's
    built-in responders: `Stream`, `Redirect`, `File`, `NamedFile`, `content`
    for manually setting Content-Types, and `Either`. In the process, showcases
    using `TempFile` for raw uploads. Also illustrates the creation of a custom,
    derived `Responder`.

  * **[`serialization`](./serialization)** - Showcases JSON and MessagePack
    (de)serialization support by implementing a CRUD-like message API in JSON
    and a simply read/echo API in MessagePack. Showcases UUID parsing support.

  * **[`state`](./state)** - Illustrates the use of request-local state and
    managed state. Uses request-local state to cache "expensive" per-request
    operations. Uses managed state to implement a simple index hit counter. Also
    uses managed state to store, retrieve, and push/pop from a concurrent queue.

  * **[`static-files`](./static-files)** - Uses `FileServer` to serve static
    files. Also creates a `second` manual yet safe version.

  * **[`templating`](./templating)** - Illustrates using `contrib` `templates`
    support with identical examples for handlebars and tera.

  * **[`testing`](./testing)** - Uses Rocket's `local` libraries to test an
    application. Showcases necessary use of the `async` `Client`. Note that all
    examples contains tests, themselves serving as examples for how to test
    Rocket applications.

  * **[`tls`](./tls)** - Illustrates configuring TLS with a variety of key pair
    kinds.
