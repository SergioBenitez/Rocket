use rocket::Request;
use rocket::response::Redirect;

use rocket_dyn_templates::{Template, handlebars, context};

use self::handlebars::{Handlebars, JsonRender};

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
struct TemplateContext<'r> {
    title: &'r str,
    name: Option<&'r str>,
    items: Vec<&'r str>,
}

#[get("/")]
pub fn index() -> Redirect {
    Redirect::to(uri!("/hbs", hello(name = "Your Name")))
}

#[get("/hello/<name>")]
pub fn hello(name: &str) -> Template {
    Template::render("hbs/index", context! {
        title: "Hello",
        name: Some(name),
        items: vec!["One", "Two", "Three"],
    })
}

#[get("/about")]
pub fn about() -> Template {
    Template::render("hbs/about.html", context! {
        title: "About",
        parent: "hbs/layout",
    })
}

#[catch(404)]
pub fn not_found(req: &Request<'_>) -> Template {
    Template::render("hbs/error/404", context! {
        uri: req.uri()
    })
}

fn wow_helper(
    h: &handlebars::Helper<'_, '_>,
    _: &handlebars::Handlebars,
    _: &handlebars::Context,
    _: &mut handlebars::RenderContext<'_, '_>,
    out: &mut dyn handlebars::Output
) -> handlebars::HelperResult {
    if let Some(param) = h.param(0) {
        out.write("<b><i>")?;
        out.write(&param.value().render())?;
        out.write("</b></i>")?;
    }

    Ok(())
}

pub fn customize(hbs: &mut Handlebars) {
    hbs.register_helper("wow", Box::new(wow_helper));
    hbs.register_template_string("hbs/about.html", r#"
        {{#*inline "page"}}

        <section id="about">
          <h1>About - Here's another page!</h1>
        </section>

        {{/inline}}
        {{> hbs/layout}}
    "#).expect("valid HBS template");
}
