use darling::{
    ast::{Data, Fields, Style},
    util::Ignored,
    FromDeriveInput, FromVariant,
};
use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{DeriveInput, Expr, Generics, Ident, Visibility};

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
    vis: Visibility,
    generics: Generics,
    data: Data<ErrorVariant, Ignored>,
}

impl ToTokens for HttpErrorOpts {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let enum_ident = &self.ident;
        let enum_vis = &self.vis;
        let (impl_generics, ty_generics, where_clause) = self.generics.split_for_impl();

        let data = match self.data.as_ref() {
            Data::Enum(val) => val,
            Data::Struct(_) => panic!("expected an error enum, not a struct"),
        };

        let error_struct_name = format_ident!("__HttpError{}", enum_ident);

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

                        #error_struct_name {
                            code: #status.as_u16(),
                            message: "Internal server error".to_string()
                        }
                    } else {
                        #error_struct_name {
                            code: #status.as_u16(),
                            message: self.to_string()
                        }
                    };

                    (value, #status)
                }
            }
        });

        let aide_impl = if cfg!(feature = "aide") {
            quote! {
                impl #impl_generics ::aide::OperationOutput for #enum_ident #ty_generics #where_clause {
                    type Inner = Self;

                    fn operation_response(
                        ctx: &mut ::aide::gen::GenContext,
                        operation: &mut ::aide::openapi::Operation
                    ) -> Option<::aide::openapi::Response> {
                        <::axum::Json<#error_struct_name> as ::aide::OperationOutput>::operation_response(
                            ctx,
                            operation
                        )
                    }

                    fn inferred_responses(
                        ctx: &mut ::aide::gen::GenContext,
                        operation: &mut ::aide::openapi::Operation
                    ) -> Vec<(Option<u16>, ::aide::openapi::Response)> {
                        Vec::new()
                    }
                }

                impl #impl_generics ::serde::Serialize for #enum_ident #ty_generics #where_clause {
                    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                    where
                        S: ::serde::Serializer
                    {
                        let (error, _) = self.into_private_error();
                        #error_struct_name::serialize(&error, serializer)
                    }
                }
            }
        } else {
            quote! {}
        };

        let schemars_derive = if cfg!(feature = "aide") {
            quote! {
                #[derive(::schemars::JsonSchema)]
            }
        } else {
            quote! {}
        };

        quote! {
            #[derive(::serde::Serialize)]
            #schemars_derive
            #enum_vis struct #error_struct_name {
                code: u16,
                message: String
            }

            impl #impl_generics #enum_ident #ty_generics #where_clause {
                fn into_private_error(&self) -> (#error_struct_name, ::axum::http::StatusCode) {
                    match self {
                        #(#matcher),*
                    }
                }
            }

            impl #impl_generics ::axum::response::IntoResponse for #enum_ident #ty_generics #where_clause {
                fn into_response(self) -> ::axum::response::Response {
                    let (error, status) = self.into_private_error();
                    let mut response = ::axum::Json(error).into_response();
                    *response.status_mut() = status;
                    response
                }
            }

            #aide_impl
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
