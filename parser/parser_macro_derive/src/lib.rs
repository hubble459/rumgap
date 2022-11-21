use proc_macro::TokenStream;
use quote::quote;
use syn;

#[proc_macro_derive(ParserDerive)]
pub fn parser_macro_derive(input: TokenStream) -> TokenStream {
    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let ast = syn::parse(input).unwrap();

    // Build the trait implementation
    impl_parser_macro(&ast)
}

fn impl_parser_macro(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let gen = quote! {
        #[async_trait::async_trait]
        impl crate::parser::Parser for #name {
            fn can_search(&self) -> Option<Vec<String>> {
                self.parser_can_search()
            }

            fn hostnames(&self) -> Vec<&'static str> {
                self.parser_hostnames()
            }

            fn rate_limit(&self) -> u32 {
                self.parser_rate_limit()
            }

            async fn images(&self, url: &reqwest::Url) -> crate::parse_error::Result<Vec<reqwest::Url>> {
                self.images_from_url(url).await
            }

            async fn manga(&self, url: reqwest::Url) -> crate::parse_error::Result<crate::model::Manga> {
                self.get_manga(url).await
            }

            async fn search(&self, keyword: String, hostnames: Vec<String>) -> crate::parse_error::Result<Vec<crate::model::SearchManga>> {
                self.do_search(keyword, hostnames).await
            }
        }
    };
    gen.into()
}