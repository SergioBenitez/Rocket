use proc_macro::TokenStream;

use devise::{DeriveGenerator, FromMeta, MapperBuild, Support, ValidatorBuild};
use devise::proc_macro2_diagnostics::SpanDiagnosticExt;
use devise::syn::{Fields, spanned::Spanned};

#[derive(Debug, FromMeta)]
struct DatabaseAttribute {
    #[meta(naked)]
    name: String,
}

const ONE_DATABASE_ATTR: &str = "`Database` derive requires exactly one \
    `#[database(\"\")] attribute`";
const ONE_UNNAMED_FIELD: &str = "`Database` derive can only be applied to \
    structs with exactly one unnamed field";

pub fn derive_database(input: TokenStream) -> TokenStream {
    DeriveGenerator::build_for(input, quote!(impl rocket_db_pools::Database))
        .support(Support::TupleStruct)
        .validator(ValidatorBuild::new()
            .struct_validate(|_, struct_| {
                if struct_.fields.len() == 1 {
                    Ok(())
                } else {
                    return Err(struct_.fields.span().error(ONE_UNNAMED_FIELD))
                }
            })
        )
        .inner_mapper(MapperBuild::new()
            .try_struct_map(|_, struct_| {
                let krate = quote_spanned!(struct_.span() => ::rocket_db_pools);
                let db_name = match DatabaseAttribute::one_from_attrs("database", &struct_.attrs)? {
                    Some(attr) => attr.name,
                    None => return Err(struct_.span().error(ONE_DATABASE_ATTR)),
                };
                let fairing_name = format!("'{}' Database Pool", db_name);

                let pool_type = match &struct_.fields {
                    Fields::Unnamed(f) => &f.unnamed[0].ty,
                    _ => unreachable!("Support::TupleStruct"),
                };

                Ok(quote_spanned! { struct_.span() =>
                    const NAME: &'static str = #db_name;
                    type Pool = #pool_type;
                    fn fairing() -> #krate::Fairing<Self> {
                        #krate::Fairing::new(#fairing_name)
                    }
                    fn pool(&self) -> &Self::Pool { &self.0 }
                })
            })
        )
        .outer_mapper(MapperBuild::new()
            .try_struct_map(|_, struct_| {
                let decorated_type = &struct_.ident;
                let pool_type = match &struct_.fields {
                    Fields::Unnamed(f) => &f.unnamed[0].ty,
                    _ => unreachable!("Support::TupleStruct"),
                };

                Ok(quote_spanned! { struct_.span() =>
                    impl From<#pool_type> for #decorated_type {
                        fn from(pool: #pool_type) -> Self {
                            Self(pool)
                        }
                    }
                })
            })
        )
        .to_tokens()
}
