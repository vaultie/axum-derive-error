use darling::{
    ast::{Data, Fields, Style},
    util::Ignored,
    FromDeriveInput, FromVariant,
};
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{DeriveInput, Expr, Generics, Ident};

#[derive(FromVariant, Debug)]
#[darling(attributes(http_error))]
struct ErrorVariant {
    ident: Ident,
    fields: Fields<()>,

    #[darling(default)]
    status: Option<Expr>,
}

#[derive(FromDeriveInput, Debug)]
#[darling(attributes(http_error))]
struct HttpErrorOpts {
    ident: Ident,
    generics: Generics,
    data: Data<ErrorVariant, Ignored>,
}

impl ToTokens for HttpErrorOpts {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let enum_ident = &self.ident;
        let (impl_generics, ty_generics, where_clause) = self.generics.split_for_impl();

        let data = match self.data.as_ref() {
            Data::Enum(val) => val,
            Data::Struct(_) => panic!("expected an error enum, not a struct"),
        };

        let matcher = data.iter().map(|variant| {
            let name = &variant.ident;
            let fields = match &variant.fields.style {
                Style::Tuple => {
                    let placeholders = variant.fields.iter().map(|_| quote! { _ });

                    quote! { (#(#placeholders),*) }
                }
                Style::Struct => quote! { { .. } },
                Style::Unit => quote! {},
            };

            let internal_error = syn::parse_quote! {
                ::axum::http::StatusCode::INTERNAL_SERVER_ERROR
            };

            let status = variant.status.as_ref().unwrap_or(&internal_error);

            quote! {
                #enum_ident :: #name #fields => {
                    let value = if !::core::cfg!(debug_assertions) && #status == #internal_error {
                        ::tracing::error!(error = %self, "internal server error");

                        ::serde_json::json!({
                            "code": #status.as_u16(),
                            "message": "Internal server error"
                        })
                    } else {
                        ::serde_json::json!({
                            "code": #status.as_u16(),
                            "message": self.to_string()
                        })
                    };

                    ::axum::Json(value).into_response()
                }
            }
        });

        quote! {
            impl #impl_generics ::axum::response::IntoResponse for #enum_ident #ty_generics #where_clause {
                fn into_response(self) -> ::axum::response::Response {
                    match self {
                        #(#matcher),*
                    }
                }
            }
        }
        .to_tokens(tokens);
    }
}

#[proc_macro_derive(HttpError, attributes(http_error))]
pub fn derive_http_error(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);

    match HttpErrorOpts::from_derive_input(&input) {
        Ok(val) => val.to_token_stream().into(),
        Err(e) => e.write_errors().into(),
    }
}
