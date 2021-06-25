use devise::{Diagnostic, FromMeta, Result, SpanWrapped, Spanned};
use devise::ext::{SpanDiagnosticExt, TypeExt};
use indexmap::{IndexSet, IndexMap};
use proc_macro2::Span;
use quote::ToTokens;

use crate::proc_macro_ext::Diagnostics;
use crate::http_codegen::{MediaType, WebSocketEvent};
use crate::attribute::param::{Parameter, Dynamic, Guard};
use crate::syn_ext::FnArgExt;
use crate::name::Name;
use crate::http::ext::IntoOwned;
use crate::http::uri::{Origin, fmt};

/// This structure represents the parsed `route` attribute and associated items.
#[derive(Debug)]
pub struct Route {
    /// The attribute: `#[get(path, ...)]`.
    pub attr: Attribute,
    /// The static and dynamic path parameters.
    pub path_params: Vec<Parameter>,
    /// The static and dynamic query parameters.
    pub query_params: Vec<Parameter>,
    /// The data guard, if any.
    pub data_guard: Option<Guard>,
    /// The request guards.
    pub request_guards: Vec<Guard>,
    /// The decorated function: the handler.
    pub handler: syn::ItemFn,
    /// The parsed arguments to the user's function.
    pub arguments: Arguments,
}

type ArgumentMap = IndexMap<Name, (syn::Ident, syn::Type)>;

#[derive(Debug)]
pub struct Arguments {
    pub span: Span,
    pub map: ArgumentMap
}

/// Convience struct to represent an isize that cannot be `isize::max_value()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct NonMax(isize);

impl FromMeta for NonMax {
    fn from_meta(meta: &devise::MetaItem) -> Result<Self> {
        let tmp = isize::from_meta(meta)?;
        if tmp < isize::max_value() {
            Ok(Self(tmp))
        } else {
            Err(Diagnostic::spanned(
                meta.span(),
                devise::Level::Error,
                "isize::max_value() is not a permitted value"
            ))
        }
    }
}

impl ToTokens for NonMax {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.0.to_tokens(tokens);
    }
}

/// The parsed `#[route(..)]` attribute.
#[derive(Debug, FromMeta)]
pub struct Attribute {
    #[meta(naked)]
    pub method: SpanWrapped<WebSocketEvent>,
    pub uri: RouteUri,
    pub data: Option<SpanWrapped<Dynamic>>,
    pub format: Option<SpanWrapped<MediaType>>,
    pub rank: Option<NonMax>,
}

/// The parsed `#[method(..)]` (e.g, `get`, `put`, etc.) attribute.
#[derive(Debug, FromMeta)]
pub struct MethodAttribute {
    #[meta(naked)]
    pub uri: RouteUri,
    pub data: Option<SpanWrapped<Dynamic>>,
    pub format: Option<SpanWrapped<MediaType>>,
    pub rank: Option<NonMax>,
}

#[derive(Debug)]
pub struct RouteUri {
    origin: Origin<'static>,
    path_span: Span,
    query_span: Option<Span>,
}

impl FromMeta for RouteUri {
    fn from_meta(meta: &devise::MetaItem) -> Result<Self> {
        let string = crate::proc_macro_ext::StringLit::from_meta(meta)?;

        let origin = Origin::parse_route(&string)
            .map_err(|e| {
                let span = string.subspan(e.index() + 1..(e.index() + 2));
                span.error(format!("invalid route URI: {}", e))
                    .help("expected URI in origin form: \"/path/<param>\"")
            })?;

        if !origin.is_normalized() {
            let normalized = origin.clone().into_normalized();
            let span = origin.path().find("//")
                .or_else(|| origin.query()
                    .and_then(|q| q.find("&&"))
                    .map(|i| origin.path().len() + 1 + i))
                .map(|i| string.subspan((1 + i)..(1 + i + 2)))
                .unwrap_or(string.span());

            return Err(span.error("route URIs cannot contain empty segments")
                .note(format!("expected \"{}\", found \"{}\"", normalized, origin)));
        }

        let path_span = string.subspan(1..origin.path().len() + 1);
        let query_span = origin.query().map(|q| {
            let len_to_q = 1 + origin.path().len() + 1;
            let end_of_q = len_to_q + q.len();
            string.subspan(len_to_q..end_of_q)
        });

        Ok(RouteUri { origin: origin.into_owned(), path_span, query_span })
    }
}

impl Route {
    pub fn upgrade_param(param: Parameter, args: &Arguments) -> Result<Parameter> {
        if !param.dynamic().is_some() {
            return Ok(param);
        }

        let param = param.take_dynamic().expect("dynamic() => take_dynamic()");
        Route::upgrade_dynamic(param, args).map(Parameter::Guard)
    }

    pub fn upgrade_dynamic(param: Dynamic, args: &Arguments) -> Result<Guard> {
        if let Some((ident, ty)) = args.map.get(&param.name) {
            Ok(Guard::from(param, ident.clone(), ty.clone()))
        } else {
            let msg = format!("expected argument named `{}` here", param.name);
            let diag = param.span().error("unused parameter").span_note(args.span, msg);
            Err(diag)
        }
    }

    pub fn from(attr: Attribute, handler: syn::ItemFn) -> Result<Route> {
        // Collect diagnostics as we proceed.
        let mut diags = Diagnostics::new();

        // Emit a warning if a `data` param was supplied for non-payload methods.
        if let Some(ref data) = attr.data {
            if !attr.method.supports_payload() {
                let msg = format!("'{}' does not typically support payloads", *attr.method);
                // FIXME(diag: warning)
                data.full_span.warning("`data` used with non-payload-supporting method")
                    .span_note(attr.method.span, msg)
                    .emit_as_item_tokens();
            }
        }

        // Check the validity of function arguments.
        let span = handler.sig.paren_token.span;
        let mut arguments = Arguments { map: ArgumentMap::new(), span };
        for arg in &handler.sig.inputs {
            if let Some((ident, ty)) = arg.typed() {
                let value = (ident.clone(), ty.with_stripped_lifetimes());
                arguments.map.insert(Name::from(ident), value);
            } else {
                let span = arg.span();
                let diag = if arg.wild().is_some() {
                    span.error("handler arguments must be named")
                        .help("to name an ignored handler argument, use `_name`")
                } else {
                    span.error("handler arguments must be of the form `ident: Type`")
                };

                diags.push(diag);
            }
        }

        // Parse and collect the path parameters.
        let (source, span) = (attr.uri.path(), attr.uri.path_span);
        let path_params = Parameter::parse_many::<fmt::Path>(source.as_str(), span)
            .map(|p| Route::upgrade_param(p?, &arguments))
            .filter_map(|p| p.map_err(|e| diags.push(e.into())).ok())
            .collect::<Vec<_>>();

        // Parse and collect the query parameters.
        let query_params = match (attr.uri.query(), attr.uri.query_span) {
            (Some(q), Some(span)) => Parameter::parse_many::<fmt::Query>(q.as_str(), span)
                .map(|p| Route::upgrade_param(p?, &arguments))
                .filter_map(|p| p.map_err(|e| diags.push(e.into())).ok())
                .collect::<Vec<_>>(),
            _ => vec![]
        };

        // Remove the `SpanWrapped` layer and upgrade to a guard.
        let data_guard = attr.data.clone()
            .map(|p| Route::upgrade_dynamic(p.value, &arguments))
            .and_then(|p| p.map_err(|e| diags.push(e.into())).ok());

        // Collect all of the declared dynamic route parameters.
        let all_dyn_params = path_params.iter().filter_map(|p| p.dynamic())
            .chain(query_params.iter().filter_map(|p| p.dynamic()))
            .chain(data_guard.as_ref().map(|g| &g.source).into_iter());

        // Check for any duplicates in the dynamic route parameters.
        let mut dyn_params: IndexSet<&Dynamic> = IndexSet::new();
        for p in all_dyn_params {
            if let Some(prev) = dyn_params.replace(p) {
                diags.push(p.span().error(format!("duplicate parameter: `{}`", p.name))
                    .span_note(prev.span(), "previous parameter with the same name here"))
            }
        }

        // Collect the request guards: all the arguments not already a guard.
        let request_guards = arguments.map.iter()
            .filter(|(name, _)| {
                let mut all_other_guards = path_params.iter().filter_map(|p| p.guard())
                    .chain(query_params.iter().filter_map(|p| p.guard()))
                    .chain(data_guard.as_ref().into_iter());

                all_other_guards.all(|g| &g.name != *name)
            })
            .enumerate()
            .map(|(index, (name, (ident, ty)))| Guard {
                source: Dynamic { index, name: name.clone(), trailing: false },
                fn_ident: ident.clone(),
                ty: ty.clone(),
            })
            .collect();

        diags.head_err_or(Route {
            attr, path_params, query_params, data_guard, request_guards,
            handler, arguments,
        })
    }
}

impl std::ops::Deref for RouteUri {
    type Target = Origin<'static>;

    fn deref(&self) -> &Self::Target {
        &self.origin
    }
}
