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
///
/// If every anchor misses (say a future rustdoc switches to absolute asset URLs)
/// the detected root is empty and the loader falls back to probing `../`,
/// `../../` and `../../../` in turn. A working anchor costs exactly one request,
/// so the probing only ever fires in the broken case.
const ASSET_BOOTSTRAP: &str = concat!("<script>\n", include_str!("bootstrap.js"), "</script>\n");

/// Optional presentation settings for an image reference.
#[derive(Default)]
struct ImageOptions {
    width: Option<String>,
    alt: Option<String>,
    caption: Option<String>,
}

/// Parse `image!("path.png", width = 600, alt = "…")`.
///
/// Returns `None` for the bare-token form (`image!(path.png)`) that older
/// versions accepted, so both spellings keep working.
fn parse_image_args(tokens: proc_macro::TokenStream) -> Option<(String, ImageOptions)> {
    use syn::parse::Parser as _;

    let parser = |input: syn::parse::ParseStream<'_>| -> Result<(String, ImageOptions)> {
        let path: LitStr = input.parse()?;
        let mut opts = ImageOptions::default();

        while input.peek(syn::Token![,]) {
            input.parse::<syn::Token![,]>()?;
            if input.is_empty() {
                break;
            }

            let key: syn::Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;

            match key.to_string().as_str() {
                "width" => {
                    let value: syn::Lit = input.parse()?;
                    opts.width = Some(match value {
                        syn::Lit::Int(int) => int.base10_digits().to_string(),
                        syn::Lit::Str(s) => s.value(),
                        _ => {
                            return Err(Error::new(
                                key.span(),
                                "width must be a number or a string such as \"80%\"",
                            ))
                        }
                    });
                }
                "alt" => {
                    let value: LitStr = input.parse()?;
                    opts.alt = Some(value.value());
                }
                "caption" => {
                    let value: LitStr = input.parse()?;
                    opts.caption = Some(value.value());
                }
                other => {
                    return Err(Error::new(
                        key.span(),
                        format!("unknown option `{other}`; expected `width`, `alt` or `caption`"),
                    ))
                }
            }
        }

        Ok((path.value(), opts))
    };

    parser.parse(tokens).ok()
}

/// Read `key = value` lines from the body of an `image` / `drawio` fence.
fn parse_inline_options<'a>(lines: impl Iterator<Item = &'a str>) -> ImageOptions {
    let mut opts = ImageOptions::default();

    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let value = value.trim().trim_matches('"').to_string();
        match key.trim() {
            "width" => opts.width = Some(value),
            "alt" => opts.alt = Some(value),
            "caption" => opts.caption = Some(value),
            _ => {}
        }
    }

    opts
}

/// Copy `img` into the documentation's `images/` directory and return the markup
/// that references it. Shared by `image!` and by the `image` / `drawio` fences of
/// the `diagram` attribute.
fn save_image(img: &str, opts: &ImageOptions) -> String {
    let dest = doc_dir().join("images").join(img);

    if let Some(prefix) = dest.parent() {
        let _ = fs::create_dir_all(prefix);
    }

    if let Err(e) = fs::copy(img, &dest) {
        report(
            "image",
            &format!(
                "could not copy into the documentation: {e} (paths are resolved relative to {}, \
                 not to the source file)",
                std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string())
            ),
            img,
        );
    }

    let alt = html_escape(opts.alt.as_deref().unwrap_or(img));
    let width = match &opts.width {
        Some(width) => format!(" width=\"{}\"", html_escape(width)),
        None => String::new(),
    };

    let tag =
        format!("<img data-rsdoc-src=\"images/{img}\" alt=\"{alt}\"{width} loading=\"lazy\"/>");

    // A caption turns the image into a figure; without one a plain paragraph
    // keeps the historical output unchanged.
    let body = match &opts.caption {
        Some(caption) => format!(
            "<figure>{tag}<figcaption>{}</figcaption></figure>",
            html_escape(caption)
        ),
        None => format!("<p>{tag}</p>"),
    };

    // The blank line matters: without it markdown treats the markup and the
    // <script> as one HTML block, which then ends at the first blank line
    // inside the script — leaving the rest of the JS to be parsed as prose.
    format!("{body}\n\n{ASSET_BOOTSTRAP}")
}

/// Insert a one-line row of links to this crate's crates.io page, docs.rs page
/// and repository, plus its version and license.
///
/// Everything comes from the package metadata Cargo sets while compiling, so
/// there is nothing to keep in sync by hand:
///
/// ```ignore
/// #![doc = rsdoc::badges!()]
/// ```
///
/// renders as:
///
/// ```text
/// crates.io · docs.rs · source · v0.4.1 · MIT OR Apache-2.0
/// ```
///
/// Links that have no metadata behind them are omitted, so a crate without a
/// `repository` field simply gets no source link.
#[proc_macro]
pub fn badges(_: proc_macro::TokenStream) -> proc_macro::TokenStream {
    handle_error(|| {
        let name = std::env::var("CARGO_PKG_NAME").unwrap_or_default();
        let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
        let repository = std::env::var("CARGO_PKG_REPOSITORY").unwrap_or_default();
        let license = std::env::var("CARGO_PKG_LICENSE").unwrap_or_default();

        let mut parts: Vec<String> = Vec::new();

        if !name.is_empty() {
            parts.push(format!("[crates.io](https://crates.io/crates/{name})"));
            parts.push(format!("[docs.rs](https://docs.rs/{name})"));
        }

        if !repository.is_empty() {
            parts.push(format!("[source]({repository})"));
        }

        if !version.is_empty() {
            parts.push(format!("v{}", html_escape(&version)));
        }

        if !license.is_empty() {
            parts.push(html_escape(&license));
        }

        if parts.is_empty() {
            return Ok(LitStr::new("", Span::call_site())
                .into_token_stream()
                .into());
        }

        // Written as markdown rather than a <p>: an HTML block would swallow any
        // heading or prose written immediately after it, which is exactly where
        // `#![doc = rsdoc::badges!()]` tends to sit.
        let output = format!("{}\n\n", parts.join(" · "));

        Ok(LitStr::new(&output, Span::call_site())
            .into_token_stream()
            .into())
    })
}

/// Escape HTML and turn `…` spans into `<code>…</code>`.
///
/// The notice body lives inside an HTML block, where markdown is not parsed, so
/// the one bit of inline formatting people actually reach for is handled here.
fn with_inline_code(text: &str) -> String {
    let escaped = html_escape(text);
    let mut out = String::with_capacity(escaped.len());
    let mut in_code = false;

    for ch in escaped.chars() {
        if ch == '`' {
            out.push_str(if in_code { "</code>" } else { "<code>" });
            in_code = !in_code;
        } else {
            out.push(ch);
        }
    }

    out
}

/// A coloured callout, for deprecation or migration notices.
///
/// ```ignore
/// #![doc = rsdoc::notice!(warning, "This crate is unmaintained, use `foo` instead.")]
/// ```
///
/// Three levels are available: `info`, `warning` and `danger`. The body is plain
/// text with two exceptions — backticks become `<code>`, and HTML is escaped, so
/// nothing you write can break the page layout.
///
/// Widths and colours are inline, so the callout looks the same in every rustdoc
/// theme without a stylesheet.
#[proc_macro]
pub fn notice(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    use syn::parse::Parser as _;

    let parser = |input: syn::parse::ParseStream<'_>| -> Result<(syn::Ident, String)> {
        let level: syn::Ident = input.parse()?;
        input.parse::<syn::Token![,]>()?;
        let text: LitStr = input.parse()?;

        if !matches!(level.to_string().as_str(), "info" | "warning" | "danger") {
            return Err(Error::new(
                level.span(),
                format!("unknown level `{level}`; expected `info`, `warning` or `danger`"),
            ));
        }

        Ok((level, text.value()))
    };

    handle_error(|| {
        let (level, text) = parser.parse(input)?;

        let (accent, background, icon, label) = match level.to_string().as_str() {
            "danger" => ("#cf222e", "#ffebe9", "&#9940;", "Important"),
            "info" => ("#0969da", "#ddf4ff", "&#8505;", "Note"),
            _ => ("#9a6700", "#fff8c5", "&#9888;", "Warning"),
        };

        let html = format!(
            "<div style=\"border-left:4px solid {accent};background:{background};\
             padding:12px 16px;margin:1em 0;border-radius:4px\">\
             <strong style=\"color:{accent}\">{icon} {label}</strong><br>\n{}</div>\n",
            with_inline_code(&text)
        );

        Ok(LitStr::new(&html, Span::call_site())
            .into_token_stream()
            .into())
    })
}

/// Attach pictures, such as PNG or SVG, you can do it this way
/// #[doc = rsdoc::image!(test.png)]
#[proc_macro]
pub fn image(img: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let (path, opts) = match parse_image_args(img.clone()) {
        Some(parsed) => parsed,
        // Older spelling: `image!(path/to/file.png)` with no quotes or options.
        None => (arg_to_path(img), ImageOptions::default()),
    };

    handle_error(|| {
        let output = save_image(&path, &opts);

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

/// Directory holding rendered output, keyed by the diagram source and the
/// renderer that produced it.
///
/// Rendering costs a network round-trip (or a JVM start), and `#[doc = ...]`
/// expands during every build — not only during `cargo doc`. Caching the result
/// on disk means a repeated build reads a file instead of asking plantuml.com
/// again.
fn cache_dir() -> std::path::PathBuf {
    doc_dir()
        .parent()
        .map(|p| p.join("rsdoc-cache"))
        .unwrap_or_else(|| std::path::PathBuf::from("./rsdoc-cache"))
}

/// Where a diagram's SVG comes from.
///
/// The variant carries enough identity to key the cache: a different jar
/// produces a different picture from the same source, so a cache keyed on the
/// source alone would keep serving the old rendering after an upgrade.
enum Renderer {
    /// A local jar, fingerprinted by path, size and modification time.
    Local {
        jar: std::path::PathBuf,
        fingerprint: String,
    },
    /// plantuml.com — or nothing at all, when `RSDOC_PLANTUML_OFFLINE` is set.
    Remote,
}

fn resolve_renderer() -> Renderer {
    match find_plantuml_jar() {
        Some((jar, fingerprint)) => Renderer::Local { jar, fingerprint },
        None => Renderer::Remote,
    }
}

fn sha1_hex(text: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.input_str(text);
    hasher.result_str()
}

/// Cache key for one diagram rendered one way.
///
/// `Remote` keeps the bare source hash — the key 0.4.0–0.4.2 wrote — so
/// upgrading does not throw away caches that are still correct. A local jar
/// mixes its fingerprint in, which is what makes replacing the jar re-render
/// instead of showing the previous picture.
fn cache_key(uml_str: &str, renderer: &Renderer) -> String {
    let source_hash = sha1_hex(uml_str);

    match renderer {
        Renderer::Remote => source_hash,
        Renderer::Local { fingerprint, .. } => {
            let mut hasher = Sha1::new();
            // The source hash is fixed width, so the pair cannot be confused
            // with a different source/fingerprint split.
            hasher.input_str(&source_hash);
            hasher.input_str(fingerprint);
            hasher.result_str()
        }
    }
}

/// Identify a jar well enough to notice that it was replaced.
///
/// Path, size and modification time: an upgrade rewrites the file, and even a
/// rebuild that keeps the size moves the mtime, so both are covered.
fn jar_fingerprint(path: &std::path::Path) -> String {
    let (len, mtime) = match fs::metadata(path) {
        Ok(meta) => {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|delta| delta.as_nanos())
                .unwrap_or(0);
            (meta.len(), mtime)
        }
        Err(_) => (0, 0),
    };

    format!("{}:{len}:{mtime}", path.display())
}

/// Whether diagrams must never leave the machine.
///
/// `RSDOC_PLANTUML_OFFLINE=1` turns a missing or broken jar into a reported
/// failure instead of a silent round-trip to plantuml.com. For an air-gapped or
/// confidential build that round-trip is exactly what the jar was meant to
/// prevent, so it must not happen behind a warning.
fn offline_enabled() -> bool {
    match std::env::var("RSDOC_PLANTUML_OFFLINE") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no"
        ),
        Err(_) => false,
    }
}

/// Locate a local `plantuml.jar` and fingerprint it for the cache key.
///
/// Checked in order:
///   1. the `RSDOC_PLANTUML_JAR` environment variable
///   2. `plantuml.jar` next to the crate's `Cargo.toml`
///   3. `plantuml.jar` in an enclosing crate or workspace root, so a workspace
///      with several members needs one copy rather than one per member
///
/// Rendering locally removes the network dependency entirely, which is what
/// issue #5 asks for. Falls back to plantuml.com when no jar is found.
fn find_plantuml_jar() -> Option<(std::path::PathBuf, String)> {
    use std::path::{Path, PathBuf};

    if let Ok(value) = std::env::var("RSDOC_PLANTUML_JAR") {
        let path = PathBuf::from(value.trim());
        if path.is_file() {
            return Some((path.clone(), jar_fingerprint(&path)));
        }
    }

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    let mut dir: &Path = &manifest;

    loop {
        let candidate = dir.join("plantuml.jar");
        if candidate.is_file() {
            return Some((candidate.clone(), jar_fingerprint(&candidate)));
        }

        // Walking up stops as soon as a directory has no `Cargo.toml`, so an
        // unrelated jar somewhere higher up the disk is never picked up.
        match dir.parent() {
            Some(parent) if parent.join("Cargo.toml").is_file() => dir = parent,
            _ => return None,
        }
    }
}

/// Render through a local `plantuml.jar` using its `-pipe` mode, which reads the
/// source from stdin and writes SVG to stdout — no temporary files involved.
fn render_with_jar(jar: &std::path::Path, uml_str: &str) -> std::result::Result<String, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("java")
        .arg("-jar")
        .arg(jar)
        .arg("-tsvg")
        .arg("-pipe")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "`java` was not found on PATH; install a JRE or remove the jar".to_string()
            } else {
                format!("could not start java: {e}")
            }
        })?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "could not open plantuml's stdin".to_string())?;
        stdin
            .write_all(uml_str.as_bytes())
            .map_err(|e| format!("could not write the diagram to plantuml: {e}"))?;
    } // closing stdin signals EOF to plantuml

    let out = child
        .wait_with_output()
        .map_err(|e| format!("waiting for plantuml failed: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // PlantUML exits with 200 on a syntax error, which reads as nonsense in
        // a message, so prefer the parsed reason whenever there is one.
        return Err(match summarize_plantuml_error(&stderr) {
            Some(detail) => detail,
            None => format!("plantuml exited with {}", out.status),
        });
    }

    let svg = String::from_utf8(out.stdout)
        .map_err(|e| format!("plantuml produced invalid UTF-8: {e}"))?;

    if svg.contains("<svg") {
        Ok(svg)
    } else {
        Err("plantuml produced no <svg> output".to_string())
    }
}

/// Return the SVG for `uml_str`, reusing the on-disk cache when possible.
/// A short single-line digest of a diagram source, used in warnings.
///
/// Without it a failing diagram in a crate with dozens of them is impossible to
/// identify: the compiler output carries no location for a `#[doc]` attribute
/// expansion. Showing the beginning of the source is enough to find it.
fn preview(src: &str) -> String {
    const LIMIT: usize = 48;

    let flat = src.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > LIMIT {
        format!("{}…", flat.chars().take(LIMIT).collect::<String>())
    } else {
        flat
    }
}

/// Escape a string for embedding in a JSON string literal.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());

    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }

    out
}

/// Report a diagram that could not be rendered.
///
/// With `RSDOC_JSON` set, each diagnostic is one JSON object per line, which lets
/// CI scripts and AI agents parse the failure instead of scraping prose. Without
/// it, this is an ordinary warning aimed at a human.
fn report(kind: &str, message: &str, preview: &str) {
    report_at("warning", kind, message, preview);
}

/// Report a condition the build should not quietly tolerate — a diagram that
/// was dropped because rendering it would have left the machine.
fn report_hard(kind: &str, message: &str, preview: &str) {
    report_at("error", kind, message, preview);
}

fn report_at(prose_level: &str, kind: &str, message: &str, preview: &str) {
    if std::env::var_os("RSDOC_JSON").is_some() {
        println!(
            r#"{{"tool":"rsdoc","level":"error","kind":"{}","message":"{}","preview":"{}"}}"#,
            json_escape(kind),
            json_escape(message),
            json_escape(preview)
        );
    } else {
        println!("{prose_level}: rsdoc failed on `{preview}` ({kind}): {message}");
    }
}

/// Pull a useful message out of PlantUML's diagnostics.
///
/// In `-pipe` mode PlantUML reports a syntax error on four lines:
///
/// ```text
/// ERROR
/// 2
/// Syntax Error?
/// Some diagram description contains errors
/// ```
///
/// and still writes an "error image" to stdout with exit code 200. Only the line
/// number and reason matter, so those two are recombined into one sentence.
fn summarize_plantuml_error(stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    // "ERROR" / "<line number>" / "<reason>"
    if lines.len() >= 3
        && lines[0].eq_ignore_ascii_case("ERROR")
        && !lines[1].is_empty()
        && lines[1].chars().all(|c| c.is_ascii_digit())
    {
        return Some(format!("syntax error at line {}: {}", lines[1], lines[2]));
    }

    // Older servers phrase it as "Error line N in file: ..." on one line.
    if let Some(line) = lines
        .iter()
        .find(|line| line.to_ascii_lowercase().contains("error line"))
    {
        return Some((*line).to_string());
    }

    lines.first().map(|line| (*line).to_string())
}

/// Why nothing was rendered in offline mode.
fn offline_refusal() -> String {
    match std::env::var("RSDOC_PLANTUML_JAR") {
        Ok(value) if !value.trim().is_empty() => format!(
            "RSDOC_PLANTUML_JAR points at `{}`, which is not a readable file, and \
             RSDOC_PLANTUML_OFFLINE is set — the diagram was not sent to plantuml.com",
            value.trim()
        ),
        _ => "no local plantuml.jar was found (`RSDOC_PLANTUML_JAR`, next to `Cargo.toml`, \
              or in the workspace root) and RSDOC_PLANTUML_OFFLINE is set — the diagram was \
              not sent to plantuml.com"
            .to_string(),
    }
}

/// Mention which jar failed, unless Java's own message already names it.
///
/// A corrupt jar makes the JVM print the whole path, and repeating it reads like
/// a stutter; a missing `java` on `PATH` says nothing about the jar, so there the
/// path is the only clue.
fn jar_note(reason: &str, jar: &std::path::Path) -> String {
    let path = jar.display().to_string();

    if reason.contains(&path) {
        String::new()
    } else {
        format!(" (jar: {path})")
    }
}

fn render_svg_cached(key: &str, uml_str: &str, renderer: &Renderer) -> String {
    let cache = cache_dir().join(format!("{key}.svg"));

    if let Ok(cached) = fs::read_to_string(&cache) {
        if cached.contains("<svg") {
            return cached;
        }
    }

    // Where to store the result. It stays under `key` unless the jar was
    // expected to answer and did not: a network fallback stored under the jar's
    // key would outlive the broken jar, so installing a JRE afterwards would
    // still serve the old picture — and suppress the warning that explains why.
    let mut fallback_key: Option<String> = None;

    let svg = match renderer {
        Renderer::Local { jar, .. } => match render_with_jar(jar, uml_str) {
            Ok(svg) => svg,
            Err(reason) => {
                if offline_enabled() {
                    report_hard(
                        "plantuml",
                        &format!(
                            "{reason}{}; RSDOC_PLANTUML_OFFLINE is set, so the diagram was not \
                             sent to plantuml.com",
                            jar_note(&reason, jar)
                        ),
                        &preview(uml_str),
                    );
                    String::new()
                } else {
                    // The jar failed, but the diagram may still render remotely.
                    report(
                        "plantuml",
                        &format!(
                            "{reason}{}, falling back to plantuml.com",
                            jar_note(&reason, jar)
                        ),
                        &preview(uml_str),
                    );
                    fallback_key = Some(cache_key(uml_str, &Renderer::Remote));
                    download_puml_svg(uml_str).unwrap_or_else(|e| {
                        report("plantuml", &e, &preview(uml_str));
                        String::new()
                    })
                }
            }
        },
        Renderer::Remote => {
            if offline_enabled() {
                report_hard("plantuml", &offline_refusal(), &preview(uml_str));
                String::new()
            } else {
                download_puml_svg(uml_str).unwrap_or_else(|e| {
                    // Only suggest the jar when the network is the problem. On a syntax
                    // error it would send the reader down the wrong path.
                    let hint = if e.contains("syntax") || e.contains("400") {
                        ""
                    } else {
                        " (a local plantuml.jar via RSDOC_PLANTUML_JAR avoids the network)"
                    };
                    report("plantuml", &format!("{e}{hint}"), &preview(uml_str));
                    String::new()
                })
            }
        }
    };

    let store_under = fallback_key.as_deref().unwrap_or(key);

    if svg.contains("<svg") {
        let dir = cache_dir();
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join(format!("{store_under}.svg")), &svg);
    }

    svg
}

fn save_plantuml(uml_str: &str) -> String {
    let source_hash = sha1_hex(uml_str);
    let renderer = resolve_renderer();
    let svg = render_svg_cached(&cache_key(uml_str, &renderer), uml_str, &renderer);

    if !svg.is_empty() {
        return format!("<p><div>{svg}</div></p>");
    }

    // Nothing rendered. The historical last resort downloads a PNG from
    // plantuml.com, which is the one thing `RSDOC_PLANTUML_OFFLINE` exists to
    // prevent — `render_svg_cached` has already reported why.
    if offline_enabled() {
        return String::new();
    }

    let puml_dir = doc_dir().join("images").join("puml_files");
    let _ = fs::create_dir_all(&puml_dir);
    let uml_file = puml_dir.join(format!("{source_hash}.png"));
    let uml_file_str = uml_file.to_string_lossy().to_string();

    let mut output = String::new();

    if save_http_png::download_puml(uml_str, &uml_file_str) {
        // The page depth varies with the module nesting level, so the relative
        // path cannot be baked in here; the bootstrap resolves it at runtime.
        output.push_str(&format!(
            "<p><img data-rsdoc-src=\"images/puml_files/{source_hash}.png\" alt=\"plantuml\"/></p>\n\n{ASSET_BOOTSTRAP}"
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

fn download_puml_svg(puml_str: &str) -> std::result::Result<String, String> {
    let compressed = deflate_bytes(puml_str.as_bytes());
    let encode64_str = save_http_png::encode64_(&compressed);
    let url = "https://www.plantuml.com/plantuml/svg/".to_string() + &encode64_str;

    let agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .build();

    let response = match agent.get(&url).call() {
        Ok(response) => response,
        // 400 means plantuml.com parsed the request and rejected the diagram,
        // which almost always means a syntax problem. Showing the encoded URL
        // (ureq's default Display) would bury that.
        Err(ureq::Error::Status(400, _)) => {
            return Err(
                "plantuml.com rejected the diagram (HTTP 400) — usually a syntax error".to_string(),
            );
        }
        Err(ureq::Error::Status(code, _)) => {
            return Err(format!("plantuml.com returned HTTP {code}"));
        }
        Err(e) => return Err(format!("could not reach plantuml.com: {e}")),
    };

    let body = response
        .into_string()
        .map_err(|e| format!("could not read the response body: {e}"))?;

    if body.contains("<svg") {
        Ok(body)
    } else {
        Err(format!(
            "plantuml.com returned {} bytes without an <svg> element",
            body.len()
        ))
    }
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

/// Diagram-type keywords Mermaid accepts as the first word of a diagram.
///
/// Matching is case-sensitive on purpose: `sequencediagram` really is a syntax
/// error in Mermaid, and catching that is half the point of this check.
const MERMAID_KINDS: &[&str] = &[
    "graph",
    "flowchart",
    "sequenceDiagram",
    "classDiagram",
    "stateDiagram",
    "stateDiagram-v2",
    "erDiagram",
    "journey",
    "gantt",
    "pie",
    "gitGraph",
    "mindmap",
    "timeline",
    "quadrantChart",
    "requirementDiagram",
    "sankey-beta",
    "xychart-beta",
    "block-beta",
    "architecture-beta",
    "packet-beta",
    "C4Context",
    "C4Container",
    "C4Component",
    "C4Dynamic",
    "C4Deployment",
];

/// Levenshtein distance, capped by the caller.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();

    for i in 1..=a.len() {
        let mut current = vec![i];
        for j in 1..=b.len() {
            let substitution = previous[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            current.push((previous[j] + 1).min(current[j - 1] + 1).min(substitution));
        }
        previous = current;
    }

    previous[b.len()]
}

/// Report the first bracket or quote problem on a line, ignoring anything inside
/// a quoted label such as `A["a [b] c"]`.
fn unbalanced(line: &str) -> Option<String> {
    let mut in_quote = false;
    let mut stack: Vec<char> = Vec::new();

    for ch in line.chars() {
        match ch {
            '"' => in_quote = !in_quote,
            _ if in_quote => {}
            '[' | '(' | '{' => stack.push(ch),
            ']' | ')' | '}' => {
                let want = match ch {
                    ']' => '[',
                    ')' => '(',
                    _ => '{',
                };
                match stack.pop() {
                    Some(open) if open == want => {}
                    _ => return Some(format!("`{ch}` closes nothing")),
                }
            }
            _ => {}
        }
    }

    if in_quote {
        return Some("unterminated quote".to_string());
    }

    stack
        .first()
        .map(|open| format!("`{open}` is never closed"))
}

/// Sanity-check Mermaid source before handing it to the browser.
///
/// A real parser runs to tens of thousands of lines, which is out of proportion
/// for a build step. These checks cover what actually goes wrong when writing a
/// diagram by hand: a misspelled type, a `graph` with no direction, unbalanced
/// brackets, and a `subgraph` without `end`.
///
/// Problems are reported but never block rendering — the browser may still draw
/// something useful, so the block is emitted either way.
fn validate_mermaid(src: &str) -> Vec<String> {
    let mut problems = Vec::new();

    let header = src
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("%%"));

    let header = match header {
        Some(header) => header,
        None => {
            problems.push("the diagram is empty".to_string());
            return problems;
        }
    };

    let keyword = header.split_whitespace().next().unwrap_or("");
    let kind = match MERMAID_KINDS.iter().find(|known| **known == keyword) {
        Some(kind) => *kind,
        None => {
            let suggestion = MERMAID_KINDS
                .iter()
                .map(|known| (*known, edit_distance(known, keyword)))
                .filter(|(_, distance)| *distance <= 2)
                .min_by_key(|(_, distance)| *distance)
                .map(|(known, _)| format!(", did you mean `{known}`?"))
                .unwrap_or_default();

            problems.push(format!("unrecognised diagram type `{keyword}`{suggestion}"));
            return problems; // nothing else is meaningful without a known grammar
        }
    };

    if kind == "graph" || kind == "flowchart" {
        let direction = header.split_whitespace().nth(1).unwrap_or("");
        if !matches!(direction, "TD" | "TB" | "BT" | "LR" | "RL") {
            problems.push(format!(
                "`{kind}` needs a direction (TD, TB, BT, LR or RL), found `{direction}`"
            ));
        }

        for (index, line) in src.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("%%") {
                continue;
            }
            if let Some(issue) = unbalanced(line) {
                // Quote the offending line so the reader (human or agent) does
                // not have to count lines to find it.
                problems.push(format!("line {}: {issue} — `{line}`", index + 1));
            }
        }

        let mut open = 0i32;
        for line in src.lines() {
            let line = line.trim();
            if line.starts_with("subgraph") {
                open += 1;
            } else if line == "end" {
                if open == 0 {
                    problems.push("`end` without a matching `subgraph`".to_string());
                } else {
                    open -= 1;
                }
            }
        }
        if open > 0 {
            problems.push(format!("{open} `subgraph` without `end`"));
        }
    }

    problems
}

fn save_mermaid(mermaid_str: &str, theme: Option<&str>) -> String {
    ensure_mermaid_asset();

    for problem in validate_mermaid(mermaid_str) {
        report("mermaid", &problem, &preview(mermaid_str));
    }

    // The theme rides on the element; the bootstrap picks the first one it finds
    // and uses it for the whole page, since Mermaid is initialised once.
    let data_theme = match theme {
        Some(theme) => format!(" data-theme=\"{}\"", html_escape(theme)),
        None => String::new(),
    };

    let mut output = String::new();
    output.push_str(&format!("<pre class=\"mermaid\"{data_theme}>\n"));
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
        let output = save_mermaid(&mermaid_str, None);

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
            Ok(data) => save_mermaid(&data, None),
            Err(e) => {
                report("mermaid", &format!("could not read the file: {e}"), &path);
                String::new()
            }
        };

        Ok(LitStr::new(&output, Span::call_site())
            .into_token_stream()
            .into())
    })
}

// ---------------------------------------------------------------------------
// Attribute macros
//
// A proc-macro crate exports every macro into a single namespace, so an
// attribute macro cannot reuse the `mermaid` name already taken by the
// function-style macro — `#[rsdoc::mermaid]` and `rsdoc::mermaid!` would
// collide. `diagram` therefore takes the attribute role and handles fenced
// blocks of any supported language.
// ---------------------------------------------------------------------------

/// Borrow the attribute list of an item, whatever kind it is.
fn item_attrs_mut(item: &mut syn::Item) -> Option<&mut Vec<syn::Attribute>> {
    use syn::Item;

    match item {
        Item::Const(i) => Some(&mut i.attrs),
        Item::Enum(i) => Some(&mut i.attrs),
        Item::Fn(i) => Some(&mut i.attrs),
        Item::Impl(i) => Some(&mut i.attrs),
        Item::Mod(i) => Some(&mut i.attrs),
        Item::Static(i) => Some(&mut i.attrs),
        Item::Struct(i) => Some(&mut i.attrs),
        Item::Trait(i) => Some(&mut i.attrs),
        Item::Type(i) => Some(&mut i.attrs),
        Item::Union(i) => Some(&mut i.attrs),
        _ => None,
    }
}

/// PlantUML's server expects the diagram wrapped in `@startuml`/`@enduml`;
/// fenced blocks and included files usually omit them.
fn wrap_plantuml(body: String) -> String {
    if body.contains("@startuml") {
        body
    } else {
        format!("@startuml\n{body}@enduml\n")
    }
}

/// Recognise a standalone `include_mmd!("path")` or `include_puml!("path")`
/// directive on its own line in a doc comment.
///
/// Large diagrams are awkward to keep inline — they drown the item they
/// document. These directives let the source live in its own file.
fn parse_include(line: &str) -> Option<(&'static str, String)> {
    let trimmed = line.trim();

    for (prefix, kind) in [("include_mmd!(", "mermaid"), ("include_puml!(", "plantuml")] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.strip_suffix(')')?.trim();
            let path = rest.strip_prefix('"')?.strip_suffix('"')?;
            return Some((kind, path.to_string()));
        }
    }

    None
}

/// Read an included diagram, resolved against the crate root rather than the
/// file the macro was written in.
fn read_include(path: &str) -> std::result::Result<String, String> {
    let base = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| "CARGO_MANIFEST_DIR is not set".to_string())?;
    let full = std::path::Path::new(&base).join(path);

    fs::read_to_string(&full).map_err(|e| format!("could not read `{}`: {e}", full.display()))
}

/// Replace fenced `mermaid` and `plantuml` blocks with rendered output. Every
/// other line is passed through untouched, so ordinary prose, lists and other
/// code fences survive.
/// Split the text after an opening fence into its language and trailing
/// attributes: `mermaid theme=forest` becomes `("mermaid", "theme=forest")`.
fn split_fence(rest: &str, fence: &'static str) -> (String, String, &'static str) {
    let mut parts = rest.trim().splitn(2, char::is_whitespace);
    let lang = parts.next().unwrap_or("").to_string();
    let attrs = parts.next().unwrap_or("").trim().to_string();

    (lang, attrs, fence)
}

/// Read `theme=forest` out of a fence's trailing attributes.
fn fence_theme(attrs: &str) -> Option<String> {
    attrs
        .split_whitespace()
        .find_map(|part| part.strip_prefix("theme="))
        .map(|value| value.trim_matches('"').to_string())
}

/// Parse the arguments of `#[rsdoc::diagram(theme = "forest")]`.
fn parse_diagram_theme(attr: proc_macro::TokenStream) -> Option<String> {
    if attr.is_empty() {
        return None;
    }

    use syn::parse::Parser as _;

    let parser = |input: syn::parse::ParseStream<'_>| -> Result<String> {
        let key: syn::Ident = input.parse()?;
        input.parse::<syn::Token![=]>()?;
        let value: LitStr = input.parse()?;

        if key != "theme" {
            return Err(Error::new(
                key.span(),
                "the attribute only accepts `theme = \"…\"`",
            ));
        }

        Ok(value.value())
    };

    match parser.parse(attr) {
        Ok(theme) => Some(theme),
        Err(e) => {
            println!("warning: rsdoc ignored the `diagram` arguments: {e}");
            None
        }
    }
}

fn transform_doc_lines(lines: &[String], theme: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].clone();

        // A standalone include directive takes the whole line.
        if let Some((kind, path)) = parse_include(&line) {
            match read_include(&path) {
                Ok(body) => {
                    let html = if kind == "mermaid" {
                        save_mermaid(&body, theme)
                    } else {
                        save_plantuml(&wrap_plantuml(body))
                    };
                    out.push(html);
                }
                Err(e) => {
                    report(kind, &e, &path);
                    out.push(line);
                }
            }
            i += 1;
            continue;
        }

        let trimmed = line.trim_start();

        // Detect an opening fence, the language, and any trailing attributes
        // such as `mermaid theme=forest`.
        let fence_info = trimmed
            .strip_prefix("```")
            .map(|rest| split_fence(rest, "```"))
            .or_else(|| {
                trimmed
                    .strip_prefix("~~~")
                    .map(|rest| split_fence(rest, "~~~"))
            });

        let (lang, fence_attrs, fence) = match fence_info {
            Some(info)
                if matches!(info.0.as_str(), "mermaid" | "plantuml" | "image" | "drawio") =>
            {
                info
            }
            _ => {
                out.push(line);
                i += 1;
                continue;
            }
        };

        // Collect the block body up to the closing fence.
        let mut body = String::new();
        i += 1;
        while i < lines.len() && !lines[i].trim_start().starts_with(fence) {
            body.push_str(&lines[i]);
            body.push('\n');
            i += 1;
        }
        if i < lines.len() {
            i += 1; // consume the closing fence
        }

        let html = match lang.as_str() {
            "mermaid" => {
                let block_theme = fence_theme(&fence_attrs);
                save_mermaid(&body, block_theme.as_deref().or(theme))
            }
            // `image` and `drawio` fences hold a path, optionally followed by
            // `width = …` or `alt = …` lines.
            "image" | "drawio" => {
                let mut rest = body.lines().map(str::trim).filter(|l| !l.is_empty());
                let path = rest.next().unwrap_or("");
                let opts = parse_inline_options(rest);

                if path.is_empty() {
                    report(
                        &lang,
                        "empty block, expected a path on the first line",
                        "(empty)",
                    );
                    String::new()
                } else {
                    save_image(path, &opts)
                }
            }
            _ => save_plantuml(&wrap_plantuml(body)),
        };

        out.push(html);
    }

    out.join("\n")
}

/// Render fenced diagram blocks found in the annotated item's documentation.
///
/// ```ignore
/// #[rsdoc::diagram]
/// /// ```mermaid
/// /// graph TD
/// ///   A --> B
/// /// ```
/// pub fn example() {}
/// ```
///
/// Both `mermaid` and `plantuml` fences are recognised. The function-style
/// macros (`rsdoc::mermaid!`, `rsdoc::plantuml!`) keep working unchanged, so
/// neither form is deprecated.
#[proc_macro_attribute]
pub fn diagram(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    use syn::Item;

    let theme = parse_diagram_theme(attr);

    let mut parsed: Item = match syn::parse(item.clone()) {
        Ok(parsed) => parsed,
        Err(_) => return item,
    };

    let attrs = match item_attrs_mut(&mut parsed) {
        Some(attrs) => attrs,
        None => return item,
    };

    let mut lines: Vec<String> = Vec::new();
    let mut others: Vec<syn::Attribute> = Vec::new();

    for attr in attrs.iter() {
        if attr.path.is_ident("doc") {
            if let Ok(Meta::NameValue(nv)) = attr.parse_meta() {
                if let Lit::Str(s) = nv.lit {
                    for line in s.value().lines() {
                        lines.push(line.to_string());
                    }
                    continue;
                }
            }
        }
        others.push(attr.clone());
    }

    if lines.is_empty() {
        return quote!(#parsed).into();
    }

    let transformed = transform_doc_lines(&lines, theme.as_deref());
    let doc_attr: syn::Attribute = syn::parse_quote!(#[doc = #transformed]);

    let mut new_attrs = Vec::with_capacity(others.len() + 1);
    new_attrs.push(doc_attr);
    new_attrs.extend(others);
    *attrs = new_attrs;

    quote!(#parsed).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn escapes_html_metacharacters() {
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("<svg>"), "&lt;svg&gt;");
        assert_eq!(html_escape("plain"), "plain");
        // Ampersand first: otherwise the escaping would double-encode.
        assert_eq!(html_escape("<&>"), "&lt;&amp;&gt;");
    }

    #[test]
    fn keeps_plain_prose() {
        let input = lines(&["Just text.", "", "More text."]);
        assert_eq!(
            transform_doc_lines(&input, None),
            "Just text.\n\nMore text."
        );
    }

    #[test]
    fn keeps_unrelated_code_fences() {
        let input = lines(&["```rust", "let x = 1;", "```"]);
        let out = transform_doc_lines(&input, None);
        assert!(out.contains("```rust"), "opening fence lost: {out}");
        assert!(out.contains("let x = 1;"), "code lost: {out}");
    }

    #[test]
    fn leaves_text_surrounding_a_foreign_block_intact() {
        let input = lines(&["Before.", "", "```rust", "fn a() {}", "```", "", "After."]);
        let out = transform_doc_lines(&input, None);
        assert!(out.starts_with("Before."), "prefix lost: {out}");
        assert!(out.ends_with("After."), "suffix lost: {out}");
    }

    #[test]
    fn ignores_an_unterminated_fence() {
        // A fence with no closing marker must not swallow the rest silently.
        let input = lines(&["text", "```rust"]);
        let out = transform_doc_lines(&input, None);
        assert!(out.contains("text"));
    }

    #[test]
    fn preview_flattens_and_truncates() {
        assert_eq!(preview("graph TD\n   A --> B"), "graph TD A --> B");

        let long = "alpha ".repeat(20);
        let p = preview(&long);
        assert!(p.ends_with('…'), "expected an ellipsis: {p}");
        assert_eq!(p.chars().count(), 49, "48 chars plus the ellipsis");
    }

    #[test]
    fn parses_include_directives() {
        assert_eq!(
            parse_include(r#"include_mmd!("diagrams/a.mmd")"#),
            Some(("mermaid", "diagrams/a.mmd".to_string()))
        );
        assert_eq!(
            parse_include(r#"   include_puml!("b.puml")   "#),
            Some(("plantuml", "b.puml".to_string()))
        );

        // Anything that is not exactly the directive stays as written.
        assert_eq!(parse_include("include_mmd!(no_quotes)"), None);
        assert_eq!(parse_include(r#"include_mmd!("a.mmd") trailing"#), None);
        assert_eq!(parse_include("ordinary prose"), None);
    }

    #[test]
    fn escapes_for_json() {
        assert_eq!(json_escape("plain"), "plain");
        assert_eq!(json_escape(r#"a"b"#), r#"a\"b"#);
        assert_eq!(json_escape("line\nbreak"), r"line\nbreak");
        assert_eq!(json_escape(r"back\slash"), r"back\\slash");
        assert_eq!(json_escape("\u{1}"), r"\u0001");
    }

    #[test]
    fn picks_the_useful_plantuml_error_line() {
        let stderr = "\n\n     \nError line 3 in file: Syntax Error?\n\
                      Some diagram description contains errors\n\n";
        let msg = summarize_plantuml_error(stderr).expect("expected a summary");
        assert!(msg.contains("Error line 3"), "got: {msg}");
        assert!(!msg.starts_with('\n'));
        assert!(
            !msg.contains("Some diagram"),
            "noise should be trimmed: {msg}"
        );
    }

    #[test]
    fn combines_the_four_line_pipe_diagnostic() {
        // This is the exact shape PlantUML 1.2024.7 prints in -pipe mode.
        let stderr = "ERROR\n2\nSyntax Error?\nSome diagram description contains errors\n";
        assert_eq!(
            summarize_plantuml_error(stderr).as_deref(),
            Some("syntax error at line 2: Syntax Error?")
        );
    }

    #[test]
    fn falls_back_to_the_first_line_when_nothing_looks_like_an_error() {
        let stderr = "\n\nsomething unexpected happened\n";
        assert_eq!(
            summarize_plantuml_error(stderr).as_deref(),
            Some("something unexpected happened")
        );
        assert_eq!(summarize_plantuml_error("\n  \n"), None);
    }

    #[test]
    fn accepts_a_well_formed_flowchart() {
        let src = "graph TD\n  A[Start] --> B{Done?}\n  B -->|yes| C[Ship]\n";
        assert_eq!(validate_mermaid(src), Vec::<String>::new());

        let seq = "sequenceDiagram\n  Alice->>Bob: hi\n";
        assert_eq!(validate_mermaid(seq), Vec::<String>::new());
    }

    #[test]
    fn suggests_a_correction_for_a_misspelled_type() {
        let problems = validate_mermaid("grap TD\n  A --> B");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("did you mean `graph`"), "{problems:?}");
    }

    #[test]
    fn rejects_case_mismatched_keywords() {
        // `sequencediagram` is not the same as `sequenceDiagram`.
        let problems = validate_mermaid("sequencediagram\n  A->>B: hi");
        assert!(
            problems[0].contains("unrecognised diagram type"),
            "{problems:?}"
        );
    }

    #[test]
    fn requires_a_direction_after_graph() {
        let problems = validate_mermaid("graph\n  A --> B");
        assert!(
            problems.iter().any(|p| p.contains("needs a direction")),
            "{problems:?}"
        );
    }

    #[test]
    fn reports_unbalanced_brackets() {
        let problems = validate_mermaid("graph TD\n  A[Start --> B[Done]");
        assert!(
            problems.iter().any(|p| p.contains("never closed")),
            "{problems:?}"
        );
    }

    #[test]
    fn ignores_brackets_inside_quoted_labels() {
        let src = "graph TD\n  A[\"a [b] c\"] --> B\n";
        assert_eq!(validate_mermaid(src), Vec::<String>::new());
    }

    #[test]
    fn reports_subgraph_without_end() {
        let src = "graph TD\n  subgraph one\n  A --> B\n";
        let problems = validate_mermaid(src);
        assert!(
            problems.iter().any(|p| p.contains("without `end`")),
            "{problems:?}"
        );
    }

    #[test]
    fn rejects_an_empty_diagram() {
        assert_eq!(
            validate_mermaid("   \n\n"),
            vec!["the diagram is empty".to_string()]
        );
    }

    #[test]
    fn parses_inline_image_options() {
        let opts = parse_inline_options(["width = 320", "alt = Diagram from a fence"].into_iter());
        assert_eq!(opts.width.as_deref(), Some("320"));
        assert_eq!(opts.alt.as_deref(), Some("Diagram from a fence"));
    }

    #[test]
    fn keeps_quotes_off_inline_option_values() {
        let opts = parse_inline_options(["width = \"80%\""].into_iter());
        assert_eq!(opts.width.as_deref(), Some("80%"));
    }

    #[test]
    fn ignores_unknown_inline_options() {
        let opts = parse_inline_options(["colour = red", "just text"].into_iter());
        assert!(opts.width.is_none());
        assert!(opts.alt.is_none());
    }

    #[test]
    fn reads_a_theme_from_fence_attributes() {
        assert_eq!(fence_theme("theme=forest").as_deref(), Some("forest"));
        assert_eq!(fence_theme("theme=\"neutral\"").as_deref(), Some("neutral"));
        assert_eq!(fence_theme("").as_deref(), None);
        assert_eq!(fence_theme("other=1").as_deref(), None);
    }

    #[test]
    fn splits_a_fence_into_language_and_attributes() {
        let (lang, attrs, fence) = split_fence(" mermaid theme=forest ", "```");
        assert_eq!(lang, "mermaid");
        assert_eq!(attrs, "theme=forest");
        assert_eq!(fence, "```");

        let (lang, attrs, _) = split_fence("graph TD", "~~~");
        assert_eq!(lang, "graph");
        assert_eq!(attrs, "TD");
    }

    // ---- local plantuml.jar -------------------------------------------------

    use std::sync::Mutex;

    /// Environment variables are process-wide while `cargo test` runs tests on
    /// threads, so everything that sets one takes this lock. Every value is
    /// restored on drop, so a failure cannot leak into the next test.
    static ENV: Mutex<()> = Mutex::new(());

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct Env {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl Env {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Env { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Env { key, previous }
        }
    }

    impl Drop for Env {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("rsdoc-unit").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    /// A jar-shaped file. Nothing reads its content until a JVM does.
    fn fake_jar(dir: &std::path::Path, bytes: usize) -> std::path::PathBuf {
        let jar = dir.join("plantuml.jar");
        fs::write(&jar, vec![0u8; bytes]).expect("write fake jar");
        jar
    }

    fn local_renderer(jar: &std::path::Path) -> Renderer {
        Renderer::Local {
            jar: jar.to_path_buf(),
            fingerprint: jar_fingerprint(jar),
        }
    }

    const DIAGRAM: &str = "@startuml\nBob -> Alice : hello\n@enduml";

    #[test]
    fn a_local_jar_gets_its_own_cache_entry() {
        let dir = scratch("cache-key");
        let jar = fake_jar(&dir, 64);

        let remote = cache_key(DIAGRAM, &Renderer::Remote);
        let local = cache_key(DIAGRAM, &local_renderer(&jar));

        // The network key stays exactly what 0.4.x wrote, so upgrading does not
        // throw those caches away.
        assert_eq!(remote, sha1_hex(DIAGRAM));
        assert_ne!(
            local, remote,
            "a jar must not share the network's cache entry"
        );
        // The same jar keeps hitting the same entry.
        assert_eq!(local, cache_key(DIAGRAM, &local_renderer(&jar)));
    }

    #[test]
    fn a_replaced_jar_invalidates_the_cache_entry() {
        let dir = scratch("cache-key-replaced");
        let jar = fake_jar(&dir, 64);

        let before = cache_key(DIAGRAM, &local_renderer(&jar));
        // A different size guarantees a different fingerprint even on a
        // filesystem with one-second mtime granularity.
        fs::write(&jar, vec![1u8; 65]).expect("replace jar");

        assert_ne!(
            before,
            cache_key(DIAGRAM, &local_renderer(&jar)),
            "an upgraded jar reused the previous rendering"
        );
    }

    #[test]
    fn finds_the_jar_through_the_environment_variable() {
        let _guard = env_lock();
        let dir = scratch("discover-env");
        let jar = fake_jar(&dir, 32);
        let _jar_env = Env::set("RSDOC_PLANTUML_JAR", &jar);

        let (found, fingerprint) = find_plantuml_jar().expect("jar from the env var");
        assert_eq!(found, jar);
        assert!(fingerprint.starts_with(&jar.display().to_string()));

        // A path that does not exist is not taken at face value.
        let _missing_env = Env::set("RSDOC_PLANTUML_JAR", dir.join("missing.jar"));
        assert_ne!(
            find_plantuml_jar().map(|(path, _)| path),
            Some(jar),
            "a missing jar was reported as found"
        );
    }

    #[test]
    fn finds_the_jar_in_a_workspace_root() {
        let _guard = env_lock();
        let root = scratch("discover-workspace");
        let member = root.join("member");
        fs::create_dir_all(&member).expect("create member");
        fs::write(root.join("Cargo.toml"), "[workspace]\n").expect("workspace manifest");
        fs::write(member.join("Cargo.toml"), "[package]\nname = \"m\"\n").expect("member manifest");
        let jar = fake_jar(&root, 48);

        let _unset = Env::unset("RSDOC_PLANTUML_JAR");
        let _manifest = Env::set("CARGO_MANIFEST_DIR", &member);

        assert_eq!(find_plantuml_jar().map(|(path, _)| path), Some(jar));

        // With no `Cargo.toml` above the member the walk stops, so an unrelated
        // jar further up the disk is never picked up.
        fs::remove_file(root.join("Cargo.toml")).expect("remove workspace manifest");
        assert_eq!(find_plantuml_jar().map(|(path, _)| path), None);
    }

    #[test]
    fn offline_refuses_to_post_the_diagram() {
        let _guard = env_lock();
        let dir = scratch("offline");

        let _offline = Env::set("RSDOC_PLANTUML_OFFLINE", "1");
        // No jar anywhere: the env var points at a directory, and the manifest
        // root below holds no `plantuml.jar` either.
        let _jar = Env::set("RSDOC_PLANTUML_JAR", &dir);
        let _manifest = Env::set("CARGO_MANIFEST_DIR", &dir);
        let _target = Env::set("CARGO_TARGET_DIR", dir.join("target"));

        let renderer = resolve_renderer();
        assert!(
            matches!(&renderer, Renderer::Remote),
            "a jar was found anyway"
        );

        let key = cache_key(DIAGRAM, &renderer);
        assert!(
            render_svg_cached(&key, DIAGRAM, &renderer).is_empty(),
            "offline mode still produced a diagram, so it reached the network"
        );
    }

    /// Number of times the stand-in `java` was started.
    #[cfg(unix)]
    fn java_calls(counter: &std::path::Path) -> usize {
        fs::read_to_string(counter)
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }

    #[cfg(unix)]
    #[test]
    fn a_local_jar_renders_through_java_and_is_cached() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = env_lock();
        let dir = scratch("render");
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).expect("create bin");
        let counter = dir.join("calls");

        // Stand-in for `java -jar plantuml.jar -tsvg -pipe`: drain stdin, count
        // the call, emit an SVG. Shell builtins only, because `PATH` below holds
        // nothing else.
        let shim = bin.join("java");
        fs::write(
            &shim,
            format!(
                "#!/bin/sh\nwhile read -r line; do :; done\n\
                 echo run >> '{}'\nprintf '<svg>local</svg>'\n",
                counter.display()
            ),
        )
        .expect("write shim");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("chmod shim");

        let jar = fake_jar(&dir, 128);
        let _jar_env = Env::set("RSDOC_PLANTUML_JAR", &jar);
        let _offline = Env::unset("RSDOC_PLANTUML_OFFLINE");
        let _target = Env::set("CARGO_TARGET_DIR", dir.join("target"));
        // The shim is the only `java` reachable, so a real JVM cannot answer.
        let _path = Env::set("PATH", &bin);

        let renderer = resolve_renderer();
        assert!(
            matches!(&renderer, Renderer::Local { .. }),
            "the jar was not discovered"
        );

        let key = cache_key(DIAGRAM, &renderer);
        assert!(render_svg_cached(&key, DIAGRAM, &renderer).contains("<svg>local</svg>"));
        assert_eq!(java_calls(&counter), 1, "the jar was never started");

        // Same source and same jar: the cached SVG answers, the JVM stays idle.
        assert!(render_svg_cached(&key, DIAGRAM, &renderer).contains("<svg>local</svg>"));
        assert_eq!(
            java_calls(&counter),
            1,
            "the cache did not prevent a second JVM start"
        );

        // A newer jar may draw the same diagram differently, so it re-renders.
        fs::write(&jar, vec![9u8; 129]).expect("replace jar");
        let upgraded = resolve_renderer();
        assert!(
            render_svg_cached(&cache_key(DIAGRAM, &upgraded), DIAGRAM, &upgraded)
                .contains("<svg>local</svg>")
        );
        assert_eq!(
            java_calls(&counter),
            2,
            "an upgraded jar reused the cached SVG"
        );
    }
}
