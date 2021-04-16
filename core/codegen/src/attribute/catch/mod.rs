mod parse;

use devise::ext::SpanDiagnosticExt;
use devise::{syn, Spanned, Result};

use crate::http_codegen::Optional;
use crate::proc_macro2::{TokenStream, Span};
use crate::syn_ext::ReturnTypeExt;
use crate::exports::*;

pub fn _catch(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream
) -> Result<TokenStream> {
    // Parse and validate all of the user's input.
    let catch = parse::Attribute::parse(args.into(), input)?;

    // Gather everything we'll need to generate the catcher.
    let user_catcher_fn = &catch.function;
    let user_catcher_fn_name = &catch.function.sig.ident;
    let vis = &catch.function.vis;
    let status_code = Optional(catch.status.map(|s| s.code));

    // Determine the number of parameters that will be passed in.
    if catch.function.sig.inputs.len() > 2 {
        return Err(catch.function.sig.paren_token.span
            .error("invalid number of arguments: must be zero, one, or two")
            .help("catchers optionally take `&Request` or `Status, &Request`"));
    }

    // This ensures that "Responder not implemented" points to the return type.
    let return_type_span = catch.function.sig.output.ty()
        .map(|ty| ty.span().into())
        .unwrap_or(Span::call_site().into());

    // Set the `req` and `status` spans to that of their respective function
    // arguments for a more correct `wrong type` error span. `rev` to be cute.
    let codegen_args = &[__req, __status];
    let inputs = catch.function.sig.inputs.iter().rev()
        .zip(codegen_args.into_iter())
        .map(|(fn_arg, codegen_arg)| match fn_arg {
            syn::FnArg::Receiver(_) => codegen_arg.respanned(fn_arg.span()),
            syn::FnArg::Typed(a) => codegen_arg.respanned(a.ty.span())
        }).rev();

    // We append `.await` to the function call if this is `async`.
    let dot_await = catch.function.sig.asyncness
        .map(|a| quote_spanned!(a.span().into() => .await));

    let catcher_response = quote_spanned!(return_type_span => {
        let ___responder = #user_catcher_fn_name(#(#inputs),*) #dot_await;
        #_response::Responder::respond_to(___responder, #__req)?
    });

    // Generate the catcher, keeping the user's input around.
    Ok(quote! {
        #user_catcher_fn

        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        /// Rocket code generated proxy structure.
        #vis struct #user_catcher_fn_name {  }

        /// Rocket code generated proxy static conversion implementation.
        impl From<#user_catcher_fn_name> for #_catcher::StaticInfo {
            fn from(_: #user_catcher_fn_name) -> #_catcher::StaticInfo {
                fn monomorphized_function<'_b>(
                    #__status: #Status,
                    #__req: &'_b #Request<'_>
                ) -> #_catcher::BoxFuture<'_b> {
                    use #_trace::Instrument as _;
                    #_Box::pin(async move {
                        let __response = #catcher_response;
                        #Response::build()
                            .status(#__status)
                            .merge(__response)
                            .ok()
                    }.instrument(#_trace::info_span!(
                        stringify!(#user_catcher_fn_name),
                        status = %#__status,
                        "Catcher: {}", stringify!(#user_catcher_fn_name)
                    )))
                }

                #_catcher::StaticInfo {
                    name: stringify!(#user_catcher_fn_name),
                    code: #status_code,
                    handler: monomorphized_function,
                }
            }
        }

        /// Rocket code generated proxy conversion implementation.
        impl From<#user_catcher_fn_name> for #Catcher {
            #[inline]
            fn from(_: #user_catcher_fn_name) -> #Catcher {
                #_catcher::StaticInfo::from(#user_catcher_fn_name {}).into()
            }
        }
    })
}

pub fn catch_attribute(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream
) -> TokenStream {
    _catch(args, input).unwrap_or_else(|d| d.emit_as_item_tokens())
}
