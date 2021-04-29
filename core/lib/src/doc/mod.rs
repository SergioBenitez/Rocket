//! Traits and structs related to automagically generating documentation for your Rocket routes

use std::{collections::HashMap, marker::PhantomData};

use rocket_http::ContentType;

mod has_schema;

#[derive(Default)]
pub struct Docs(HashMap<ContentType, DocContent>);

#[derive(Default)]
pub struct DocContent {
    title: Option<String>,
    description: Option<String>,
    content_type: Option<String>,
}

pub struct Resolve<T: ?Sized>(PhantomData<T>);

pub trait Documented {
    fn docs() -> Docs;
}

trait Undocumented {
    fn docs() -> Docs {
        Docs::default()
    }
}

impl<T: ?Sized> Undocumented for T { }

impl<T: Documented + ?Sized> Resolve<T> {
    pub const DOCUMENTED: bool = true;

    pub fn docs() -> Docs {
        T::docs()
    }
}

// impl<T: Documented + ?Sized> Documented for Json<T> {
//     fn docs() -> Docs {
//         Docs {
//             content_type: Some("application/json".to_string()),
//             ..Self::docs()
//         }
//     }
// }
