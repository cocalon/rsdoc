//! End-to-end checks that need a real browser.
//!
//! Each test builds a page shaped like rustdoc output — complete with a
//! `static.files/` reference — drops in the same `bootstrap.js` that rsdoc
//! injects, and lets Chrome render it. Nothing here shells out to `cargo`, so the
//! tests run in a couple of seconds.
//!
//! When no Chrome binary can be found the tests print a notice and pass, so a
//! machine without a browser still gets a green `cargo test`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A 1x1 transparent PNG.
const PIXEL: &[u8] = include_bytes!("../examples/assets/pixel.png");

/// Locate a Chrome or Chromium binary, trying the usual install paths.
fn chrome_binary() -> Option<PathBuf> {
    const UNIX: &[&str] = &[
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ];

    for path in UNIX {
        let candidate = Path::new(path);
        if candidate.is_file() {
            return Some(candidate.to_path_buf());
        }
    }

    for variable in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(variable) {
            let candidate = Path::new(&base).join("Google/Chrome/Application/chrome.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

/// Turn a filesystem path into a `file://` URL.
fn file_url(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");

    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

/// Render a page and return the resulting DOM.
fn dump_dom(chrome: &Path, page: &Path) -> String {
    let output = Command::new(chrome)
        .args([
            "--headless=new",
            "--disable-gpu",
            "--no-sandbox",
            "--hide-scrollbars",
            "--virtual-time-budget=8000",
            "--dump-dom",
            &file_url(page),
        ])
        .output()
        .expect("failed to start chrome");

    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Fresh directory for one test, cleared first so runs do not influence each other.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("rsdoc-e2e").join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Write a page that looks like rustdoc output.
///
/// `root_prefix` is what the `static.files` reference is prefixed with — the
/// bootstrap derives the documentation root from exactly that, which is the
/// behaviour under test.
fn write_page(dir: &Path, sub: &str, body: &str, root_prefix: &str) -> PathBuf {
    let page_dir = dir.join(sub);
    fs::create_dir_all(&page_dir).expect("create page dir");

    let html = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
         <link rel=\"stylesheet\" href=\"{root}static.files/rustdoc-abc.css\">\
         </head><body><main>{body}</main><script>\n{js}\n</script></body></html>",
        root = root_prefix,
        body = body,
        js = include_str!("../src/bootstrap.js"),
    );

    let page = page_dir.join("index.html");
    fs::write(&page, html).expect("write page");
    page
}

#[test]
fn mermaid_renders_into_an_svg() {
    let Some(chrome) = chrome_binary() else {
        eprintln!("no Chrome found; skipping browser test");
        return;
    };

    let dir = scratch("mermaid");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mermaid.min.js"),
        dir.join("mermaid.min.js"),
    )
    .expect("copy mermaid.min.js");

    // The page sits one level down, so the root prefix must be "../".
    let page = write_page(
        &dir,
        "sub",
        "<pre class=\"mermaid\">graph TD\n  A[Start] --> B[Done]</pre>",
        "../",
    );

    let dom = dump_dom(&chrome, &page);
    assert!(
        dom.contains("<svg"),
        "mermaid produced no <svg>; first 400 bytes of the DOM:\n{}",
        &dom[..dom.len().min(400)]
    );
    assert!(
        dom.contains("Start") && dom.contains("Done"),
        "the diagram rendered but its labels are missing"
    );
}

#[test]
fn image_paths_resolve_from_a_nested_page() {
    let Some(chrome) = chrome_binary() else {
        eprintln!("no Chrome found; skipping browser test");
        return;
    };

    let dir = scratch("images");
    fs::create_dir_all(dir.join("images")).expect("create images dir");
    fs::write(dir.join("images/test.png"), PIXEL).expect("write pixel");

    // Depth 1: the bootstrap should resolve "images/test.png" as "../images/test.png".
    let page = write_page(
        &dir,
        "sub",
        "<img data-rsdoc-src=\"images/test.png\" alt=\"pixel\">",
        "../",
    );

    let dom = dump_dom(&chrome, &page);
    assert!(
        dom.contains("src=\"../images/test.png\""),
        "expected the asset to resolve one level up; DOM:\n{}",
        &dom[..dom.len().min(400)]
    );
}

#[test]
fn wide_content_is_constrained_by_the_injected_style() {
    let Some(chrome) = chrome_binary() else {
        eprintln!("no Chrome found; skipping browser test");
        return;
    };

    let dir = scratch("style");
    let page = write_page(&dir, "", "<p>plain page, no diagrams</p>", "");

    let dom = dump_dom(&chrome, &page);
    assert!(
        dom.contains("max-width:100%"),
        "the constraining style was not injected"
    );
}
