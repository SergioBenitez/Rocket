#![recursion_limit="256"]

#![warn(rust_2018_idioms)]

//! # `rocket_databases` - Code Generation
//!
//! This crate implements the code generation portion of the `rocket_databases`
//! crate.

#[macro_use] extern crate quote;

mod database;

use proc_macro::TokenStream;

/// Defines a database type and implements [`Database`] on it.
///
/// ```ignore
/// #[derive(Database)]
/// #[database("database_name")]
/// struct Db(PoolType);
/// ```
///
/// `PoolType` must implement [`Pool`].
///
/// This macro generates the following code, implementing the [`Database`] trait
/// on the struct. Custom implementations of `Database` should usually also
/// start with roughly this code:
///
/// ```ignore
/// impl Database for Db {
///     const NAME: &'static str = "config_name";
///     type Pool = PoolType;
///     fn fairing() -> Fairing<Self> { Fairing::new(|p| Self(p)) }
///     fn pool(&self) -> &Self::Pool { &self.0 }
/// }
/// ```
#[proc_macro_derive(Database, attributes(database))]
pub fn derive_database(input: TokenStream) -> TokenStream {
    crate::database::derive_database(input)
}
