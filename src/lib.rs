#![doc = include_str!("../README.md")]
#![warn(rust_2018_idioms)]
use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use syn::{
    self,
    parse::{Parse, ParseStream},
    parse_macro_input, AttrStyle, Attribute, Error, Lit, LitStr, Meta, MetaNameValue, Result,
};
use std::fs;

mod save_http_png;

use crypto::digest::Digest;
use crypto::sha1::Sha1;

/// An `Attribute`, recognized as a doc comment or not.
#[derive(Clone)]
enum MaybeDocAttr {
    /// A doc comment attribute.
    ///
    /// The first `Attribute` only specifies the surround tokens.
    ///
    /// `MetaNameValue::lit` must be a `Lit::Str(_)`.
    Doc(Attribute, MetaNameValue),
    /// An unrecognized attribute that we don't care.
    Other(Attribute),
}

impl MaybeDocAttr {
    fn from_attribute(attr: Attribute) -> Result<Self> {
        
        if attr.path.is_ident("doc") {
            let meta = attr.parse_meta()?;

            if let Meta::NameValue(nv) = meta {
                if let Lit::Str(_) = nv.lit {
                    Ok(MaybeDocAttr::Doc(attr, nv))
                } else {
                    Err(Error::new(nv.lit.span(), "doc comment must be a string"))
                }
            } else {
                // Ignore unrecognized form
                Ok(MaybeDocAttr::Other(attr))
            }
        } else {
            Ok(MaybeDocAttr::Other(attr))
        }
    }
}

impl ToTokens for MaybeDocAttr {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            MaybeDocAttr::Doc(attr, nv) => {
                attr.pound_token.to_tokens(tokens);
                if let AttrStyle::Inner(ref b) = attr.style {
                    b.to_tokens(tokens);
                }
                attr.bracket_token.surround(tokens, |tokens| {
                    nv.to_tokens(tokens);
                });
            }
            MaybeDocAttr::Other(attr) => attr.to_tokens(tokens),
        }
    }
}

impl Into<Attribute> for MaybeDocAttr {
    /// The mostly-lossless conversion to `Attribute`.
    fn into(self) -> Attribute {
        match self {
            MaybeDocAttr::Doc(mut attr, nv) => {
                let lit = nv.lit;
                attr.tokens = quote! { = #lit };
                attr
            }
            MaybeDocAttr::Other(attr) => attr,
        }
    }
}

enum StrOrDocAttrs {
    Str(LitStr),
    Attrs(Vec<syn::Attribute>),
}

impl Parse for StrOrDocAttrs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        if let Ok(lit_str) = input.parse() {
            Ok(Self::Str(lit_str))
        } else {
            // `#[doc = ...]` sequence
            let mut attrs = Attribute::parse_inner(input)?;
            attrs.extend(Attribute::parse_outer(input)?);
            Ok(Self::Attrs(attrs))
        }
    }
}



#[proc_macro]
pub fn image(img:proc_macro::TokenStream) -> proc_macro::TokenStream {
    let img = img.to_string().replace(" / ", "/");
    let path_s = "./target/doc/images/".to_string() + &img;
    let path = std::path::Path::new(path_s.as_str());
    let prefix = path.parent().unwrap();
    std::fs::create_dir_all(prefix).unwrap();

    // 拷贝文件
    if let Err( es ) = fs::copy(&img, path_s)
    {
        println!("{:?}", es);
    }

    handle_error(|| {
        let output = format!("</p><img src=\"../images/{}\"/>", img);
  
        Ok(LitStr::new(&output, Span::call_site())
            .into_token_stream()
            .into())
    })
}

#[proc_macro]
pub fn plantuml_file(img:proc_macro::TokenStream) -> proc_macro::TokenStream {
    let img = img.to_string().replace(" / ", "/");

    handle_error(|| {
        
        let ret = fs::read_to_string(&img);
        if let Ok(data) = ret{
            let output = save_plantuml(&data);
        
            //text_proc.finalize()?;
            //println!("Out: {}", output);
            Ok(LitStr::new(&output, Span::call_site())
                .into_token_stream()
                .into())
        }
        else{
            Ok(LitStr::new("", Span::call_site())
                .into_token_stream()
                .into())
        }
    })
}

fn save_plantuml(uml_str:&str) -> String{
    let mut output = String::new();

    let mut hasher = Sha1::new();

    hasher.input_str(&uml_str);
    

     let uml_file_name = &("./target/doc/images/puml_files/".to_string() + hasher.result_str().as_str() + ".png");
    // println!("{}",uml_file_name);

    if true == save_http_png::download_puml(&uml_str, uml_file_name)
    {
        output.push_str(&("</p><img src = \"".to_string() + "../images/puml_files/" + hasher.result_str().as_str() + ".png" + "\" />\n"));
    }
    else
    {
        println!("warning: failed to download the planduml picture, use dynamic hyperlink instead");
        output.push_str("<base href=\"https://www.plantuml.com\" />\n");
        output.push_str("<img id = \"theimg\" />\n");
        output.push_str("<script src=\"https://plantuml.com/synchro2.min.js\"></script>\n");
        output.push_str("<script>\n");
        output.push_str("function compress(s) {\n");
        output.push_str("//UTF8\n");
        output.push_str("s = unescape(encodeURIComponent(s));\n");
        output.push_str("var arr = [];\n");
        output.push_str("for (var i = 0; i < s.length; i++)\n");
        output.push_str("arr.push(s.charCodeAt(i));\n");
        output.push_str("var compressed = new Zopfli.RawDeflate(arr).compress();\n");
        output.push_str("document.getElementById(\'theimg\').src = \"//www.plantuml.com/plantuml/png/\" + encode64_(compressed);\n");
        output.push_str("}\n");
        output.push_str("let result = (function() {/*\n");

        output.push_str(&uml_str);

        output.push_str("*/}).toString().split('\\n').slice(1,-1).join('\\n');\n");
        output.push_str("window.onload = compress(result);\n");
        output.push_str("</script>\n");
    }
    output
}
/// Render ASCII-diagram code blocks in a Markdown-formatted string literal or
/// zero or more `#[doc = ...]` attributes as SVG images.
///
/// See [the module-level documentation](../index.html) for more.
#[proc_macro]
pub fn plantuml(tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input: StrOrDocAttrs = parse_macro_input!(tokens);
    let (mut iter1, mut iter2);
    let iter: &mut dyn Iterator<Item = Result<LitStr>> = match input {
        StrOrDocAttrs::Str(s) => {
            iter1 = std::iter::once(Ok(s));
            &mut iter1
        }
        StrOrDocAttrs::Attrs(attrs) => {
            iter2 = attrs
                .into_iter()
                .map(|attr| match MaybeDocAttr::from_attribute(attr)? {
                    MaybeDocAttr::Doc(
                        _,
                        syn::MetaNameValue {
                            lit: syn::Lit::Str(s),
                            ..
                        },
                    ) => Ok(s),
                    MaybeDocAttr::Doc(attr, _) | MaybeDocAttr::Other(attr) => {
                        Err(Error::new_spanned(
                            &attr,
                            "only `#[doc = ...]` attributes or a string literal are allowed here",
                        ))
                    }
                });
            &mut iter2
        }
    };

    handle_error(|| {
        
        let mut uml_str = String::new();
        for lit_str in iter {
            let lit_str = lit_str?;
            let st = lit_str.value();
            uml_str.push_str((st + "\n").as_str());
        }

        let output = save_plantuml(&uml_str);
        
        //text_proc.finalize()?;
        //println!("Out: {}", output);
        Ok(LitStr::new(&output, Span::call_site())
            .into_token_stream()
            .into())
    })
}

fn handle_error(cb: impl FnOnce() -> Result<proc_macro::TokenStream>) -> proc_macro::TokenStream {
    match cb() {
        Ok(tokens) => tokens,
        Err(e) => e.to_compile_error().into(),
    }
}
