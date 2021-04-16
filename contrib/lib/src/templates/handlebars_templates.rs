use serde::Serialize;

use crate::templates::{Engine, TemplateInfo};

pub use crate::templates::handlebars::Handlebars;

impl Engine for Handlebars<'static> {
    const EXT: &'static str = "hbs";

    fn init(templates: &[(&str, &TemplateInfo)]) -> Option<Handlebars<'static>> {
        let mut hb = Handlebars::new();
        for &(name, info) in templates {
            let path = &info.path;
            if let Err(e) = hb.register_template_file(name, path) {
                error_span!("template_error", template = %name, "Error in Handlebars template",).in_scope(|| {
                    info!(template.error = %e);
                    info!(template.path = %path.to_string_lossy());
                });
                return None;
            }
        }

        Some(hb)
    }

    fn render<C: Serialize>(&self, name: &str, context: C) -> Option<String> {
        if self.get_template(name).is_none() {
            error!(template = %name, "Handlebars template does not exist.");
            return None;
        }

        match Handlebars::render(self, name, &context) {
            Ok(string) => Some(string),
            Err(error) => {
                error!(template = %name, %error, "Error rendering Handlebars template");
                None
            }
        }
    }
}
