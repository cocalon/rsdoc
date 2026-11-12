#![doc = include_str!("../README.md")]
#![warn(rust_2018_idioms)]
use deflate::deflate_bytes;
use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use std::{fs, time::Duration};
use syn::{
    self,
    parse::{Parse, ParseStream},
    parse_macro_input, AttrStyle, Attribute, Error, Lit, LitStr, Meta, MetaNameValue, Result,
};

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

impl From<MaybeDocAttr> for Attribute {
    /// The mostly-lossless conversion to `Attribute`.
    fn from(val: MaybeDocAttr) -> Self {
        match val {
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

/// Resolve a macro argument into a filesystem path.
///
/// Accepts both a string literal (`"diagrams/flow.mmd"`) and the historical
/// bare form (`diagrams/flow.mmd`). The bare form arrives as separate tokens,
/// so the ` / ` separator that `TokenStream::to_string` inserts has to be glued
/// back together.
fn arg_to_path(tokens: proc_macro::TokenStream) -> String {
    if let Ok(lit) = syn::parse::<LitStr>(tokens.clone()) {
        return lit.value();
    }

    tokens.to_string().replace(" / ", "/")
}

/// Locate the directory rustdoc writes into.
///
/// `./target/doc` is only correct when the crate is built standalone with the
/// default target dir. It breaks under `CARGO_TARGET_DIR` and inside a
/// workspace, where `target/` sits at the workspace root instead of next to the
/// crate. Prefer Cargo's own environment variables and walk upwards from the
/// manifest until a `target` directory appears.
fn doc_dir() -> std::path::PathBuf {
    use std::env;
    use std::path::PathBuf;

    if let Ok(dir) = env::var("CARGO_TARGET_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("doc");
        }
    }

    if let Ok(manifest) = env::var("CARGO_MANIFEST_DIR") {
        let start = PathBuf::from(&manifest);
        let mut cur = start.clone();
        loop {
            let candidate = cur.join("target");
            if candidate.is_dir() {
                return candidate.join("doc");
            }
            if !cur.pop() {
                break;
            }
        }
        return start.join("target").join("doc");
    }

    PathBuf::from("./target/doc")
}

/// Injected next to every generated asset reference.
///
/// rustdoc computes the path from each page back to the documentation root and
/// rewrites it whenever the output layout changes — that is why a hard-coded
/// `../images/…` or `../../images/…` is always wrong for some page depth. Rather
/// than guessing, this resolves the root the same way rustdoc itself must, then
/// rewrites every `data-rsdoc-src` element and starts mermaid when the page holds
/// a diagram.
///
/// The anchors are tried in order of reliability:
///   1. `static.files/` — rustdoc >= 1.65 puts its CSS/JS there
///   2. `rustdoc.css` at the doc root — older releases
///   3. the `Source` link, which always points at `<root>/src/…`
const ASSET_BOOTSTRAP: &str = r#"<script>
(function () {
  if (window.__rsdocBooted) { return; }
  window.__rsdocBooted = true;

  function detectRoot() {
    var refs = document.querySelectorAll(
      'link[href*="static.files"], script[src*="static.files"]'
    );
    for (var i = 0; i < refs.length; i++) {
      var u = refs[i].getAttribute("href") || refs[i].getAttribute("src") || "";
      var at = u.indexOf("static.files");
      if (at > 0) { return u.slice(0, at); }
    }

    var css = document.querySelector('link[rel="stylesheet"][href*="rustdoc"]');
    if (css) {
      var h = css.getAttribute("href") || "";
      var slash = h.lastIndexOf("/");
      return slash >= 0 ? h.slice(0, slash + 1) : "";
    }

    var src = document.querySelector('a.src[href*="/src/"]');
    if (src) {
      var s = src.getAttribute("href") || "";
      var at2 = s.indexOf("src/");
      if (at2 > 0) { return s.slice(0, at2); }
    }

    return "";
  }

  function bootMermaid(root) {
    var theme = localStorage.getItem("rustdoc-theme");
    var dark = (theme === "dark" || theme === "ayu");
    var s = document.createElement("script");
    s.src = root + "mermaid.min.js";
    s.onload = function () {
      window.mermaid.initialize({ startOnLoad: false, theme: dark ? "dark" : "default" });
      window.mermaid.run();
    };
    s.onerror = function () { console.error("rsdoc: could not load " + s.src); };
    document.head.appendChild(s);
  }

  function resolve() {
    var root = detectRoot();

    var nodes = document.querySelectorAll("[data-rsdoc-src]");
    for (var i = 0; i < nodes.length; i++) {
      var el = nodes[i];
      var abs = root + el.getAttribute("data-rsdoc-src");
      if (el.tagName === "IMG") { el.src = abs; }
      else { el.setAttribute("href", abs); }
    }

    if (document.querySelector("pre.mermaid")) { bootMermaid(root); }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", resolve);
  } else {
    resolve();
  }
})();
</script>
"#;

///Attach pictures, such as PNG or SVG, you can do it this way
/// #[doc = rsdoc::image!(test.png)]
#[proc_macro]
pub fn image(img: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let img = arg_to_path(img);
    let path_s = doc_dir().join("images").join(&img);
    let prefix = path_s.parent().unwrap();
    std::fs::create_dir_all(prefix).unwrap();

    // copy file
    if let Err(es) = fs::copy(&img, path_s) {
        println!("{:?}", es);
    }

    handle_error(|| {
        // The blank line matters: without it markdown treats the <img> and the
        // <script> as one HTML block, which then ends at the first blank line
        // inside the script — leaving the rest of the JS to be parsed as prose.
        let output = format!(
            "<p><img data-rsdoc-src=\"images/{img}\" alt=\"{img}\"/></p>\n\n{ASSET_BOOTSTRAP}"
        );

        Ok(LitStr::new(&output, Span::call_site())
            .into_token_stream()
            .into())
    })
}

/// Directly attach the puml file stored as a file
/// #[doc = rsdoc::plantuml_file!(test.puml)]
#[proc_macro]
pub fn plantuml_file(img: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let img = arg_to_path(img);

    handle_error(|| {
        let ret = fs::read_to_string(&img);
        if let Ok(data) = ret {
            let output = save_plantuml(&data);
            Ok(LitStr::new(&output, Span::call_site())
                .into_token_stream()
                .into())
        } else {
            Ok(LitStr::new("", Span::call_site())
                .into_token_stream()
                .into())
        }
    })
}

fn save_plantuml(uml_str: &str) -> String {
    let mut output = String::new();

    let mut hasher = Sha1::new();

    hasher.input_str(uml_str);

    let hash = hasher.result_str();
    let puml_dir = doc_dir().join("images").join("puml_files");
    let _ = fs::create_dir_all(&puml_dir);
    let uml_file = puml_dir.join(format!("{}.png", hash));
    let uml_file_str = uml_file.to_string_lossy().to_string();

    let svg = download_puml_svg(uml_str);

    if !svg.is_empty() {
        output.push_str(&("<p><div>".to_string() + svg.as_str() + "</div></p>"));
    } else if save_http_png::download_puml(uml_str, &uml_file_str) {
        // The page depth varies with the module nesting level, so the relative
        // path cannot be baked in here; the bootstrap resolves it at runtime.
        output.push_str(&format!(
            "<p><img data-rsdoc-src=\"images/puml_files/{hash}.png\" alt=\"plantuml\"/></p>\n\n{ASSET_BOOTSTRAP}"
        ));
    } else {
        println!("warning: failed to download the plantuml picture, use dynamic hyperlink instead");
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

        output.push_str(uml_str);

        output.push_str("*/}).toString().split('\\n').slice(1,-1).join('\\n');\n");
        output.push_str("window.onload = compress(result);\n");
        output.push_str("</script>\n");
    }
    output
}
/// Flatten a string literal or a sequence of `#[doc = ...]` attributes into a
/// single source string. Shared by the PlantUML and Mermaid entry points.
fn collect_source(input: StrOrDocAttrs) -> Result<String> {
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

    let mut out = String::new();
    for lit_str in iter {
        out.push_str(&(lit_str?.value() + "\n"));
    }
    Ok(out)
}

/// Render code blocks in a PlantUML string
#[proc_macro]
pub fn plantuml(tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input: StrOrDocAttrs = parse_macro_input!(tokens);

    handle_error(|| {
        let uml_str = collect_source(input)?;
        let output = save_plantuml(&uml_str);

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

fn download_puml_svg(puml_str: &str) -> String {
    let compressed = deflate_bytes(puml_str.as_bytes());
    let encode64_str = save_http_png::encode64_(&compressed);
    let url = "http://www.plantuml.com/plantuml/svg/".to_string() + &encode64_str;

    let agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .build();

    let ret = match agent.get(&url).call() {
        Ok(response) => response.into_string(),
        Err(_) => return String::default(),
    };

    ret.unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Mermaid
//
// Mermaid has no server-side renderer comparable to plantuml.com, and shipping
// a JS runtime in a proc-macro is not an option. Instead the diagram source is
// emitted as a `<pre class="mermaid">` block and the diagram is drawn by the
// browser. The mermaid.js bundle below is embedded into the crate and copied
// into `target/doc/` at build time, so the generated documentation renders with
// no network access at all.
// ---------------------------------------------------------------------------

/// Bundled mermaid.js UMD build (v11). `include_bytes!` keeps it inside the
/// proc-macro binary; it is written to `target/doc/` on demand.
const MERMAID_JS: &[u8] = include_bytes!("../assets/mermaid.min.js");

/// Copy the bundled mermaid.js into the documentation root when it is missing or
/// stale.
fn ensure_mermaid_asset() {
    let doc_dir = doc_dir();
    if let Err(e) = fs::create_dir_all(&doc_dir) {
        println!(
            "warning: rsdoc could not create {}: {}",
            doc_dir.display(),
            e
        );
        return;
    }

    let dest = doc_dir.join("mermaid.min.js");

    // Writing 3.4 MiB on every macro invocation would be wasteful; an intact
    // copy of the same length is good enough to skip the work.
    if let Ok(meta) = fs::metadata(&dest) {
        if meta.len() == MERMAID_JS.len() as u64 {
            return;
        }
    }

    if let Err(e) = fs::write(&dest, MERMAID_JS) {
        println!("warning: rsdoc could not write {}: {}", dest.display(), e);
    }
}

/// Escape the characters that HTML would otherwise treat as markup. The browser
/// hands the *text* content of the `<pre>` to mermaid, so this round-trips back
/// to the original source.
fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn save_mermaid(mermaid_str: &str) -> String {
    ensure_mermaid_asset();

    let mut output = String::new();
    output.push_str("<pre class=\"mermaid\">\n");
    output.push_str(&html_escape(mermaid_str));
    output.push_str("\n</pre>\n\n");
    output.push_str(ASSET_BOOTSTRAP);
    output
}

/// Render Mermaid diagrams in doc comments.
///
/// The bundled mermaid.js is copied to `target/doc/mermaid.min.js`, so the
/// resulting documentation needs no network access.
///
/// ```ignore
/// #[doc = rsdoc::mermaid!(
/// /// graph TD
/// ///   A[Start] --> B[Done]
/// )]
/// pub fn example() {}
/// ```
#[proc_macro]
pub fn mermaid(tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input: StrOrDocAttrs = parse_macro_input!(tokens);

    handle_error(|| {
        let mermaid_str = collect_source(input)?;
        let output = save_mermaid(&mermaid_str);

        Ok(LitStr::new(&output, Span::call_site())
            .into_token_stream()
            .into())
    })
}

/// Attach a Mermaid diagram stored in a `.mmd` file.
///
/// ```ignore
/// #[doc = rsdoc::mermaid_file!("diagrams/flow.mmd")]
/// pub fn example() {}
/// ```
#[proc_macro]
pub fn mermaid_file(path: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let path = arg_to_path(path);

    handle_error(|| {
        let output = match fs::read_to_string(&path) {
            Ok(data) => save_mermaid(&data),
            Err(e) => {
                println!("warning: rsdoc could not read {}: {}", path, e);
                String::new()
            }
        };

        Ok(LitStr::new(&output, Span::call_site())
            .into_token_stream()
            .into())
    })
}
