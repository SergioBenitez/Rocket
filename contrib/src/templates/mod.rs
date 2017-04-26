extern crate serde;
extern crate serde_json;
extern crate glob;

#[cfg(feature = "tera_templates")]
mod tera_templates;

#[cfg(feature = "handlebars_templates")]
mod handlebars_templates;

#[macro_use] mod macros;

use self::serde::Serialize;
use self::glob::glob;

use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::fmt;

use rocket::config::{self, ConfigError};
use rocket::response::{self, Content, Responder};
use rocket::http::{ContentType, Status};

/// The Template type implements generic support for template rendering in
/// Rocket.
///
/// Templating in Rocket works by first discovering all of the templates inside
/// the template directory. The template directory is configurable via the
/// `template_dir` configuration parameter and defaults to `templates/`. The
/// path set in `template_dir` should be relative to the Rocket configuration
/// file. See the [configuration chapter](https://rocket.rs/guide/overview/#configuration)
/// of the guide for more information on configuration.
///
/// Templates are discovered according to their extension. At present, this
/// library supports the following templates and extensions:
///
/// * **Tera**: `.tera`
/// * **Handlebars**: `.hbs`
///
/// Any file that ends with one of these extension will be discovered and
/// rendered with the corresponding templating engine. The name of the template
/// will be the path to the template file relative to `template_dir` minus at
/// most two extensions. The following are examples of template names (on the
/// right) given that the template is at the path on the left.
///
///   * `{template_dir}/index.html.hbs` => index
///   * `{template_dir}/index.tera` => index
///   * `{template_dir}/index.hbs` => index
///   * `{template_dir}/dir/index.hbs` => dir/index
///   * `{template_dir}/dir/index.html.tera` => dir/index
///   * `{template_dir}/index.template.html.hbs` => index.template
///   * `{template_dir}/subdir/index.template.html.hbs` => subdir/index.template
///
/// The recommended naming scheme is to use two extensions: one for the file
/// type, and one for the template extension. This means that template
/// extensions should look like: `.html.hbs`, `.html.tera`, `.xml.hbs`, etc.
///
/// Templates are rendered with the `render` method. The method takes in the
/// name of a template and a context to render the template with. The context
/// can be any type that implements `Serialize` from
/// [Serde](https://github.com/serde-rs/json) and would serialize to an `Object`
/// value.
///
/// # Usage
///
/// To use, add the `handlebars_templates` feature, the `tera_templates`
/// feature, or both, to the `rocket_contrib` dependencies section of your
/// `Cargo.toml`:
///
/// ```toml,ignore
/// [dependencies.rocket_contrib]
/// version = "*"
/// default-features = false
/// features = ["handlebars_templates", "tera_templates"]
/// ```
///
/// The Template type implements Rocket's `Responder` trait, so it can be
/// returned from a request handler directly:
///
/// ```rust,ignore
/// #[get("/")]
/// fn index() -> Template {
///     let context = ...;
///     Template::render("index", &context)
/// }
/// ```
// Fields are: (optionally rendered template, template extension)
#[derive(Debug)]
pub struct Template(Option<String>, Option<String>);

#[derive(Debug)]
pub struct TemplateInfo {
    /// The complete path, including `template_dir`, to this template.
    full_path: PathBuf,
    /// The complete path, without `template_dir`, to this template.
    path: PathBuf,
    /// The extension for the engine of this template.
    extension: String,
    /// The extension before the engine extension in the template, if any.
    data_type: Option<String>
}

const DEFAULT_TEMPLATE_DIR: &'static str = "templates";

lazy_static! {
    static ref TEMPLATES: HashMap<String, TemplateInfo> = discover_templates();
    static ref TEMPLATE_DIR: PathBuf = {
        let default_dir_path = config::active().ok_or(ConfigError::NotFound)
            .map(|config| config.root().join(DEFAULT_TEMPLATE_DIR))
            .map_err(|_| {
                warn_!("No configuration is active!");
                warn_!("Using default template directory: {:?}", DEFAULT_TEMPLATE_DIR);
            })
            .unwrap_or(PathBuf::from(DEFAULT_TEMPLATE_DIR));

        config::active().ok_or(ConfigError::NotFound)
            .and_then(|config| config.get_str("template_dir"))
            .map(|user_dir| PathBuf::from(user_dir))
            .map_err(|e| {
                if !e.is_not_found() {
                    e.pretty_print();
                    warn_!("Using default directory '{:?}'", default_dir_path);
                }
            })
            .unwrap_or(default_dir_path)
    };
}

impl Template {
    /// Render the template named `name` with the context `context`. The
    /// `context` can be of any type that implements `Serialize`. This is
    /// typically a `HashMap` or a custom `struct`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::collections::HashMap;
    /// use rocket_contrib::Template;
    ///
    /// // Create a `context`. Here, just an empty `HashMap`.
    /// let mut context = HashMap::new();
    ///
    /// # context.insert("test", "test");
    /// let template = Template::render("index", &context);
    /// # assert_eq!(template.to_string(), "");
    pub fn render<S, T>(name: S, context: &T) -> Template
        where S: AsRef<str>, T: Serialize
    {
        let name = name.as_ref();
        let template = TEMPLATES.get(name);
        if template.is_none() {
            let names: Vec<_> = TEMPLATES.keys().map(|s| s.as_str()).collect();
            error_!("Template '{}' does not exist.", name);
            info_!("Known templates: {}", names.join(","));
            info_!("Searched in '{:?}'.", *TEMPLATE_DIR);
            return Template(None, None);
        }

        // Keep this set in-sync with the `engine_set` invocation. The macro
        // `return`s a `Template` if the extenion in `template` matches an
        // engine in the set. Otherwise, control will fall through.
        render_set!(name, template.unwrap(), context,
            "tera_templates" => tera_templates,
            "handlebars_templates" => handlebars_templates,
        );

        unreachable!("A template extension was discovered but not rendered.")
    }
}

/// Returns a response with the Content-Type derived from the template's
/// extension and a fixed-size body containing the rendered template. If
/// rendering fails, an `Err` of `Status::InternalServerError` is returned.
impl Responder<'static> for Template {
    fn respond(self) -> response::Result<'static> {
        let content_type = match self.1 {
            Some(ref ext) => ContentType::from_extension(ext),
            None => ContentType::HTML
        };

        match self.0 {
            Some(render) => Content(content_type, render).respond(),
            None => Err(Status::InternalServerError)
        }
    }
}

/// Renders `self`. If the template cannot be rendered, nothing is written.
impl fmt::Display for Template {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            Some(ref render) => render.fmt(f),
            None => Ok(())
        }
    }
}

/// Removes the file path's extension or does nothing if there is none.
fn remove_extension<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    let stem = match path.file_stem() {
        Some(stem) => stem,
        None => return path.to_path_buf()
    };

    match path.parent() {
        Some(parent) => parent.join(stem),
        None => PathBuf::from(stem)
    }
}

/// Splits a path into a relative path from TEMPLATE_DIR, a name that
/// may be used to identify the template, and the template's data type.
fn split_path(path: &Path) -> (PathBuf, String, Option<String>) {
    let rel_path = path.strip_prefix(&*TEMPLATE_DIR).unwrap().to_path_buf();
    let path_no_ext = remove_extension(&rel_path);
    let data_type = path_no_ext.extension();
    let mut name = remove_extension(&path_no_ext).to_string_lossy().into_owned();

    // Ensure template name consistency on Windows systems
    if cfg!(windows) {
        name = name.replace("\\", "/");
    }

    (rel_path, name, data_type.map(|d| d.to_string_lossy().into_owned()))
}


/// Returns a HashMap of `TemplateInfo`'s for all of the templates in
/// `TEMPLATE_DIR`. Templates are all files that match one of the extensions for
/// engine's in `engine_set`.
///
/// **WARNING:** This function should be called ONCE from a SINGLE THREAD.
fn discover_templates() -> HashMap<String, TemplateInfo> {
    // Keep this set in-sync with the `render_set` invocation.
    let engines = engine_set![
        "tera_templates" => tera_templates,
        "handlebars_templates" => handlebars_templates,
    ];

    let mut templates: HashMap<String, TemplateInfo> = HashMap::new();
    for &(ext, _) in &engines {
        let mut glob_path: PathBuf = TEMPLATE_DIR.join("**").join("*");
        glob_path.set_extension(ext);
        for path in glob(glob_path.to_str().unwrap()).unwrap().filter_map(Result::ok) {
            let (rel_path, name, data_type) = split_path(&path);
            if let Some(info) = templates.get(&*name) {
                warn_!("Template name '{}' does not have a unique path.", name);
                info_!("Existing path: {:?}", info.full_path);
                info_!("Additional path: {:?}", path);
                warn_!("Using existing path for template '{}'.", name);
                continue;
            }

            templates.insert(name, TemplateInfo {
                full_path: path.to_path_buf(),
                path: rel_path,
                extension: ext.to_string(),
                data_type: data_type,
            });
        }
    }

    for &(ext, register_fn) in &engines {
        let named_templates = templates.iter()
            .filter(|&(_, i)| i.extension == ext)
            .map(|(k, i)| (k.as_str(), i))
            .collect::<Vec<_>>();

        unsafe { register_fn(&*named_templates); }
    };

    templates
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Combines a `relative_path` and the `TEMPLATE_DIR` path into a full path.
    fn template_path(relative_path: &str) -> PathBuf {
        let mut path = PathBuf::from(&*TEMPLATE_DIR);
        path.push(relative_path);
        path
    }

    /// Returns the template system name, given a relative path to a file.
    fn relative_path_to_name(relative_path: &str) -> String {
        let path = template_path(relative_path);
        let (_, name, _) = split_path(&path);
        name
    }

    #[test]
    fn template_path_index_html() {
        let path = template_path("index.html.hbs");
        let (rel_path, name, data_type) = split_path(&path);

        assert_eq!(rel_path.to_string_lossy(), "index.html.hbs");
        assert_eq!(name, "index");
        assert_eq!(data_type, Some("html".to_owned()));
    }

    #[test]
    fn template_path_subdir_index_html() {
        let path = template_path("subdir/index.html.hbs");
        let (rel_path, name, data_type) = split_path(&path);

        assert_eq!(rel_path.to_string_lossy(), "subdir/index.html.hbs");
        assert_eq!(name, "subdir/index");
        assert_eq!(data_type, Some("html".to_owned()));
    }

    #[test]
    fn template_path_doc_examples() {
        assert_eq!(relative_path_to_name("index.html.hbs"), "index");
        assert_eq!(relative_path_to_name("index.tera"), "index");
        assert_eq!(relative_path_to_name("index.hbs"), "index");
        assert_eq!(relative_path_to_name("dir/index.hbs"), "dir/index");
        assert_eq!(relative_path_to_name("dir/index.html.tera"), "dir/index");
        assert_eq!(relative_path_to_name("index.template.html.hbs"),
                   "index.template");
        assert_eq!(relative_path_to_name("subdir/index.template.html.hbs"),
                   "subdir/index.template");
    }
}
