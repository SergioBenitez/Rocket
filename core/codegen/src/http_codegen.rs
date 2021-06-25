
use quote::ToTokens;
use devise::{FromMeta, MetaItem, Result, ext::{Split2, PathExt, SpanDiagnosticExt}};
use proc_macro2::{TokenStream, Span};

use crate::http;

#[derive(Debug)]
pub struct ContentType(pub http::ContentType);

#[derive(Debug)]
pub struct Status(pub http::Status);

#[derive(Debug)]
pub struct MediaType(pub http::MediaType);

#[derive(Debug, Copy, Clone)]
pub struct Method(pub http::Method);

#[derive(Debug, Copy, Clone)]
pub enum WebSocketEvent {
    Http(Method),
    WebSocketJoin,
    WebSocketMessage,
    WebSocketLeave,
}

#[derive(Clone, Debug)]
pub struct Optional<T>(pub Option<T>);

#[derive(Debug)]
pub struct Origin<'a>(pub &'a http::uri::Origin<'a>, pub Span);

#[derive(Debug)]
pub struct Absolute<'a>(pub &'a http::uri::Absolute<'a>, pub Span);

#[derive(Debug)]
pub struct Authority<'a>(pub &'a http::uri::Authority<'a>, pub Span);

#[derive(Debug)]
pub struct Reference<'a>(pub &'a http::uri::Reference<'a>, pub Span);

#[derive(Debug)]
pub struct Asterisk(pub http::uri::Asterisk, pub Span);

impl FromMeta for Status {
    fn from_meta(meta: &MetaItem) -> Result<Self> {
        let num = usize::from_meta(meta)?;
        if num < 100 || num >= 600 {
            return Err(meta.value_span().error("status must be in range [100, 599]"));
        }

        Ok(Status(http::Status::new(num as u16)))
    }
}

impl ToTokens for Status {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let code = self.0.code;
        tokens.extend(quote!(rocket::http::Status { code: #code }));
    }
}

impl FromMeta for ContentType {
    fn from_meta(meta: &MetaItem) -> Result<Self> {
        http::ContentType::parse_flexible(&String::from_meta(meta)?)
            .map(ContentType)
            .ok_or(meta.value_span().error("invalid or unknown content type"))
    }
}

impl ToTokens for ContentType {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let http_media_type = self.0.media_type().clone();
        let media_type = MediaType(http_media_type);
        tokens.extend(quote!(::rocket::http::ContentType(#media_type)));
    }
}

impl FromMeta for MediaType {
    fn from_meta(meta: &MetaItem) -> Result<Self> {
        let mt = http::MediaType::parse_flexible(&String::from_meta(meta)?)
            .ok_or(meta.value_span().error("invalid or unknown media type"))?;

        if !mt.is_known() {
            // FIXME(diag: warning)
            meta.value_span()
                .warning(format!("'{}' is not a known media type", mt))
                .emit_as_item_tokens();
        }

        Ok(MediaType(mt))
    }
}

impl ToTokens for MediaType {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let (top, sub) = (self.0.top().as_str(), self.0.sub().as_str());
        let (keys, values) = self.0.params().map(|(k, v)| (k.as_str(), v)).split2();
        let http = quote!(::rocket::http);

        tokens.extend(quote! {
            #http::MediaType::const_new(#top, #sub, &[#((#keys, #values)),*])
        });
    }
}

const VALID_METHODS_STR: &str = "`GET`, `PUT`, `POST`, `DELETE`, `HEAD`, \
    `PATCH`, `OPTIONS`, `JOIN`, `MESSAGE`, `LEAVE`";

const VALID_METHODS: &[http::Method] = &[
    http::Method::Get, http::Method::Put, http::Method::Post,
    http::Method::Delete, http::Method::Head, http::Method::Patch,
    http::Method::Options,
];

impl FromMeta for Method {
    fn from_meta(meta: &MetaItem) -> Result<Self> {
        let span = meta.value_span();
        let help_text = format!("method must be one of: {}", VALID_METHODS_STR);

        if let MetaItem::Path(path) = meta {
            if let Some(ident) = path.last_ident() {
                let method = ident.to_string().parse()
                    .map_err(|_| span.error("invalid HTTP method").help(&*help_text))?;

                if !VALID_METHODS.contains(&method) {
                    return Err(span.error("invalid HTTP method for route handlers")
                               .help(&*help_text));
                }

                return Ok(Method(method));
            }
        }

        Err(span.error(format!("expected identifier, found {}", meta.description()))
                .help(&*help_text))
    }
}

impl FromMeta for WebSocketEvent {
    fn from_meta(meta: &MetaItem) -> Result<Self> {
        let span = meta.value_span();
        let help_text = format!("method must be one of: {}", VALID_METHODS_STR);
        if let Ok(m) = Method::from_meta(meta) {
            Ok(Self::from(m))
        } else {
            if let MetaItem::Path(path) = meta {
                if let Some(ident) = path.last_ident() {
                    return match ident.to_string().to_uppercase().as_str() {
                        "JOIN" => Ok(Self::WebSocketJoin),
                        "MESSAGE" => Ok(Self::WebSocketMessage),
                        "LEAVE" => Ok(Self::WebSocketLeave),
                        _ => Err(span.error("Invalid Method").help(&*help_text)),
                    };
                }
            }
            Err(span.error(format!("expected identifier, found {}", meta.description()))
                    .help(&*help_text))
        }
    }
}

impl From<Method> for WebSocketEvent {
    fn from(m: Method) -> Self {
        Self::Http(m)
    }
}

impl From<http::Method> for WebSocketEvent {
    fn from(m: http::Method) -> Self {
        Self::Http(Method(m))
    }
}

impl ToTokens for Method {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let method_tokens = match self.0 {
            http::Method::Get => quote!(::rocket::http::Method::Get),
            http::Method::Put => quote!(::rocket::http::Method::Put),
            http::Method::Post => quote!(::rocket::http::Method::Post),
            http::Method::Delete => quote!(::rocket::http::Method::Delete),
            http::Method::Options => quote!(::rocket::http::Method::Options),
            http::Method::Head => quote!(::rocket::http::Method::Head),
            http::Method::Trace => quote!(::rocket::http::Method::Trace),
            http::Method::Connect => quote!(::rocket::http::Method::Connect),
            http::Method::Patch => quote!(::rocket::http::Method::Patch),
        };

        tokens.extend(method_tokens);
    }
}

impl ToTokens for WebSocketEvent {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let method_tokens = match self {
            Self::Http(_) => quote!(::rocket::route::WebSocketEvent::None),
            Self::WebSocketJoin => quote!(::rocket::route::WebSocketEvent::Join),
            Self::WebSocketMessage => quote!(::rocket::route::WebSocketEvent::Message),
            Self::WebSocketLeave => quote!(::rocket::route::WebSocketEvent::Leave),
        };

        tokens.extend(method_tokens);
    }
}

impl WebSocketEvent {
    pub fn http_method(self) -> Method {
        match self {
            Self::Http(m) => m,
            _ => Method(http::Method::Get),
        }
    }

    pub fn supports_payload(&self) -> bool {
        match self {
            Self::Http(m) => m.0.supports_payload(),
            _ => true,
        }
    }

    pub fn is_websocket(&self) -> bool {
        !matches!(self, Self::Http(_))
    }
}

impl std::fmt::Display for WebSocketEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(m) => write!(f, "{}", m.0),
            Self::WebSocketJoin => write!(f, "Join"),
            Self::WebSocketMessage => write!(f, "Message"),
            Self::WebSocketLeave => write!(f, "Leave"),
        }
    }
}

impl<T: ToTokens> ToTokens for Optional<T> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        use crate::exports::{_Some, _None};
        use devise::Spanned;

        let opt_tokens = match self.0 {
            Some(ref val) => quote_spanned!(val.span() => #_Some(#val)),
            None => quote!(#_None)
        };

        tokens.extend(opt_tokens);
    }
}

impl ToTokens for Origin<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let (origin, span) = (self.0, self.1);
        let origin = origin.clone().into_normalized();
        define_spanned_export!(span => _uri);

        let path = origin.path().as_str();
        let query = Optional(origin.query().map(|q| q.as_str()));
        tokens.extend(quote_spanned! { span =>
            #_uri::Origin::const_new(#path, #query)
        });
    }
}

impl ToTokens for Absolute<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let (absolute, span) = (self.0, self.1);
        define_spanned_export!(span => _uri);
        let absolute = absolute.clone().into_normalized();

        let scheme = absolute.scheme();
        let auth = Optional(absolute.authority().map(|a| Authority(a, span)));
        let path = absolute.path().as_str();
        let query = Optional(absolute.query().map(|q| q.as_str()));
        tokens.extend(quote_spanned! { span =>
            #_uri::Absolute::const_new(#scheme, #auth, #path, #query)
        });
    }
}

impl ToTokens for Authority<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let (authority, span) = (self.0, self.1);
        define_spanned_export!(span => _uri);

        let user_info = Optional(authority.user_info());
        let host = authority.host();
        let port = Optional(authority.port());
        tokens.extend(quote_spanned! { span =>
            #_uri::Authority::const_new(#user_info, #host, #port)
        });
    }
}

impl ToTokens for Reference<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let (reference, span) = (self.0, self.1);
        define_spanned_export!(span => _uri);
        let reference = reference.clone().into_normalized();

        let scheme = Optional(reference.scheme());
        let auth = Optional(reference.authority().map(|a| Authority(a, span)));
        let path = reference.path().as_str();
        let query = Optional(reference.query().map(|q| q.as_str()));
        let frag = Optional(reference.fragment().map(|f| f.as_str()));
        tokens.extend(quote_spanned! { span =>
            #_uri::Reference::const_new(#scheme, #auth, #path, #query, #frag)
        });
    }
}

impl ToTokens for Asterisk {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        define_spanned_export!(self.1 => _uri);
        tokens.extend(quote_spanned!(self.1 => #_uri::Asterisk));
    }
}
