# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.2] - 2026-11-14

### Added

- `notice!` renders a coloured callout in three levels — `info`, `warning`,
  `danger` — for deprecation and migration notes. Styling is inline so it works
  in every rustdoc theme without a stylesheet; backticks in the body become
  `<code>`.
- `badges!` emits a one-line row of links — `crates.io · docs.rs · source ·
  vX.Y.Z · LICENSE` — built from the package metadata, so nothing has to be kept
  in sync by hand. Metadata that is absent is simply skipped.
- `caption` on images, which wraps the picture in a `<figure>` with a
  `<figcaption>`. Every image also carries `loading="lazy"` now.
- Mermaid themes: `#[rsdoc::diagram(theme = "forest")]` pins a palette for the
  page, and a fence can override it (`mermaid theme=neutral`). Without either,
  diagrams keep following rustdoc's light/dark setting.
- End-to-end browser tests. They build a rustdoc-shaped page, drop in the real
  bootstrap, render it in headless Chrome and assert on the DOM: Mermaid produces
  an `<svg>`, an image referenced from a nested page resolves one level up, and
  the width-constraining style is present. Previously every "it renders" claim
  was checked by hand and nothing in CI would have caught a break.
- `image!` accepts `width` and `alt`:
  `image!("diagrams/arch.png", width = 600)`. The same options work as
  `key = value` lines inside an `image` or `drawio` fence, and `width` takes
  either a pixel count or a string such as `"80%"`. Previously every picture was
  rendered at its natural size with the file name as alt text.
- `llms.txt` at the crate root: syntax, gotchas and the diagnostics format on one
  page, for AI agents and anything that wants to understand the crate without
  reading the source.
- A structural check for Mermaid source, run at build time. It catches a
  misspelled or unsupported diagram type (with a suggestion), a `graph` without a
  direction, unbalanced brackets or quotes, and a `subgraph` without `end`.
  Previously these stayed invisible until someone opened the rendered docs, since
  Mermaid is drawn by the browser and a broken diagram does not fail the build.
- `RSDOC_JSON=1` turns every diagnostic into one JSON object per line, so CI
  scripts and AI agents can parse failures instead of scraping prose.

### Changed

- The injected bootstrap now lives in `src/bootstrap.js` and is pulled in with
  `include_str!`, so the end-to-end tests load the exact bytes the macro emits
  instead of a copy that could drift.
- Mermaid bracket errors quote the offending line:
  ``line 2: `[` is never closed — `A[Start --> B[Done]` ``.
- `include_mmd!("path")` and `include_puml!("path")` directives on their own line
  in a doc comment pull a diagram from a file instead of writing it inline. Paths
  resolve against the crate root, so a large diagram no longer drowns the item it
  documents.

### Fixed

- Render warnings now quote the diagram that failed — and the jar path when a
  local jar was involved. Previously the output was
  `could not render the diagram: <reason>` with no way to tell which one, which
  is useless in a crate with dozens of them.
- PlantUML's stderr is captured instead of discarded, so a syntax error reads
  `syntax error at line 3: Syntax Error?` rather than a bare exit code. (PlantUML
  exits with 200 on syntax errors, which was previously the entire message.)
- A diagram rejected by plantuml.com (HTTP 400) no longer suggests installing a
  local jar; that hint only appears when the network is really the problem.

## [0.4.1] - 2026-11-12

### Changed

- The README now opens with a rendered example: a fenced `mermaid` block and the
  diagram rustdoc produces from it, so the crate page shows the result instead of
  only describing it.

## [0.4.0] - 2026-11-12

### Added

- **Offline Mermaid support.** `rsdoc::mermaid!` and `rsdoc::mermaid_file!` emit
  a `<pre class="mermaid">` block that the browser draws with a bundled
  `mermaid.js`. The script is copied into the documentation root at build time,
  so the generated docs need no network access at all — no CDN, no server
  round-trip, and it works on an air-gapped machine.
- **`#[rsdoc::diagram]` attribute macro.** Renders fenced `mermaid`, `plantuml`,
  `image` and `drawio` blocks found in doc comments, leaving the surrounding
  prose untouched. The function-style macros keep working exactly as before.
- **Local PlantUML rendering.** A `plantuml.jar` discovered through
  `RSDOC_PLANTUML_JAR` or placed next to `Cargo.toml` is driven through
  `java -jar plantuml.jar -tsvg -pipe`, removing the network dependency.
  Falls back to plantuml.com when the jar is absent or fails.
- **Render cache.** Rendered diagrams are cached under
  `target/rsdoc-cache/<sha1>.svg`. Since `#[doc = ...]` expands on every build,
  not just `cargo doc`, this turns a repeated build from a network round-trip
  into a file read.
- `examples/` covering every macro.
- A GitHub Actions workflow running fmt, clippy, build, test, doc, and markup
  assertions on the generated HTML.

### Fixed

- **`image!` produced broken links on crate-root pages.** Every reference used a
  hard-coded `../../images/…`, which only resolves for items nested one module
  deep; on `crate/fn.x.html` the browser looked under `target/images/` and the
  image 404ed. Paths are now resolved at runtime from rustdoc's own
  `static.files` reference, with `rustdoc.css` and the `Source` link as fallbacks
  and a depth probe as a last resort, so any nesting depth works.
- **Workspace and `CARGO_TARGET_DIR` builds wrote assets to the wrong place.**
  The output directory is now derived from `CARGO_TARGET_DIR`, or by walking up
  from `CARGO_MANIFEST_DIR` until a `target/` directory is found.
- **Quoted paths were rejected.** `image!("test.png")` kept its quotes, so the
  file read failed; only the bare `image!(test.png)` form worked. Both forms are
  accepted now, for `image!`, `plantuml_file!` and `mermaid_file!`.
- `plantuml!` no longer emits an unpaired `</p>` (issue #3).
- Endpoints moved from `http://` to `https://`.

### Changed

- **Removed the unused `clap` dependency.** `save_http_png.rs` defined a clap
  `Parser` that nothing ever called, forcing every dependent project to compile
  clap and clap-derive. Build time for this crate drops from about 14s to 5.8s.
- The documented MSRV is now 1.71 (required by `ureq`), replacing the stale 1.54
  claim.
- `syn` gained the `full` feature so the attribute macro can parse items.

## [0.3.1] - 2026-11-12

### Fixed

- `plantuml!` emitted `</p><div>…</div>` — a closing tag with no opening
  counterpart — which made `cargo doc` warn about an unopened HTML tag
  (issue #3). It now emits `<p><div>…</div></p>`.

### Changed

- The `license` field uses the SPDX form `MIT OR Apache-2.0`.
- Description and keywords mention Mermaid.

## [0.3.0] - 2024-09-30

### Added

- PlantUML diagrams are embedded as SVG rather than PNG, which also fixed
  relative-path problems inside workspaces (contributed by wai-t).

### Changed

- Clippy and formatting cleanups (contributed by janstarke).

## [0.2.0] - 2024-01-17

- Internal updates.

## [0.1.0] - 2023-02-19

- First release: `plantuml!`, `plantuml_file!` and `image!`.
