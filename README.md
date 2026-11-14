# rsdoc

This crate provides procedural macros that turn diagrams written in doc comments
into images inside your rustdoc output.

- **[Mermaid]**: drawn by the browser from a bundled `mermaid.js`. Fully offline,
  no CDN and no server round-trip.
- **[`plantuml`]**: rendered as SVG (falling back to PNG) through `plantuml.com`.
- **[`drawio`] or any image**: copied into the output directory and referenced from
  the generated page.

*Requires Rust version 1.71 or later.*

## How it compares

Two other crates solve overlapping problems. An honest comparison:

|  | rsdoc | [aquamarine] | [simple-mermaid] |
|---|---|---|---|
| Maintenance | active | **unmaintained** | active |
| Mermaid | bundled, renders offline | bundled | loaded from a CDN |
| PlantUML | local jar or plantuml.com | — | — |
| Drawio / images | ✓ | — | — |
| Inline code block | ✓ | ✓ | — (files only) |
| Diagram from file | `include_mmd!` / `include_puml!` | `include_mmd!` | ✓ |
| Dependencies | six | several | **zero** |

**Use simple-mermaid** if Mermaid from files is all you need and you want zero
dependencies. It is smaller, and it does one thing well.

**Use rsdoc** if you need PlantUML, Drawio, diagrams written inline, or Mermaid
that renders with no network access at all — for intranet docs, archives, or
anything that has to survive without a CDN.

[aquamarine]: https://crates.io/crates/aquamarine
[simple-mermaid]: https://crates.io/crates/simple-mermaid

[Mermaid]: https://mermaid.js.org/
[`drawio`]: https://drawio-app.com/
[`plantuml`]: https://www.plantuml.com/
          <https://raw.githubusercontent.com/cocalon/rsdoc/main/rsdoc_example.png>
<img src="https://raw.githubusercontent.com/cocalon/rsdoc/main/rsdoc_example.png"
   style="border: 10px solid rgba(192, 192, 192, 0.15)">

## Usage

Add the following line to `Cargo.toml`.

```toml
[dependencies]
rsdoc = "0.4.1"
```

### `#[rsdoc::diagram]` — recommended

Annotate the item and write an ordinary fenced block. The `mermaid`, `plantuml`,
`image` and `drawio` fences are recognised; everything else — prose, lists,
other code fences — is passed through untouched.

````ignore
#[rsdoc::diagram]
/// Pipeline overview:
///
/// ```mermaid
/// graph TD
///   A --> B
/// ```
///
/// ```drawio
/// diagrams/architecture.drawio.svg
/// ```
pub fn pipeline() {}
````

This is equivalent to the function-style macros below, but it reads like plain
documentation and your editor highlights the diagram source.

<img src="https://raw.githubusercontent.com/cocalon/rsdoc/main/example-mermaid.png"
   alt="A fenced mermaid block, and what rustdoc renders from it" width="820">

> The attribute is called `diagram`, not `mermaid`, because a proc-macro crate
> exports every macro into a single namespace: `#[rsdoc::mermaid]` would collide
> with the existing `rsdoc::mermaid!`.

Large diagrams are easier to keep in their own file. Write a directive on its
own line and the file is read from the crate root:

````ignore
#[rsdoc::diagram]
/// Architecture overview:
///
/// include_mmd!("docs/diagrams/architecture.mmd")
///
/// include_puml!("docs/diagrams/sequence.puml")
pub fn architecture() {}
````

### `mermaid!`

Render a [Mermaid] diagram. Write the source inline as doc comments:

    #[doc = rsdoc::mermaid!(
    /// graph TD
    ///   A[Start] --> B{Done?}
    ///   B -->|yes| C[Ship it]
    ///   B -->|no| D[Fix]
    )]
    pub fn test_function(){}

Or keep large diagrams in their own `.mmd` file:

    #[doc = rsdoc::mermaid_file!("diagrams/flow.mmd")]

The diagram is emitted as a `<pre class="mermaid">` block and drawn by the
browser. `mermaid.js` (v11) ships with this crate and is copied to
`target/doc/mermaid.min.js` during the build, so the generated documentation
renders **without any network access** — no CDN, no server request, works on an
air-gapped machine. The theme follows rustdoc's light/dark setting.

### `plantuml!`

Wrap doc comments with `#[doc = plantuml!(...)]`. Use `plantuml` code blocks to write uml diagrams.
In this way, you can directly use the Alt+D key to browse the UML diagram in the source code after installing the PlantUML plug-in

    #[doc = rsdoc::plantuml!(
    /// @startuml
    /// !theme cyborg-outline
    /// Bob -> Alice : hello
    /// @enduml
    )]
    pub fn test_function(){}

By default PlantUML diagrams are rendered by requesting `plantuml.com`, so this
macro does need network access at build time. To render locally instead, put a
`plantuml.jar` next to `Cargo.toml` or point the `RSDOC_PLANTUML_JAR` environment
variable at one — rsdoc then runs `java -jar plantuml.jar -tsvg -pipe` and nothing
leaves the machine.

### `plantuml_file!`
Or you can directly attach the puml file stored as a file
    
    #[doc = rsdoc::plantuml_file!(test.puml)]

If your file is placed in a folder under the project directory, you can input the relative path directly like that.

    #[doc = rsdoc::plantuml_file!(pumls/test.puml)]

### `image!`

Attach any picture — PNG, JPEG or SVG:

    #[doc = rsdoc::image!(test.png)]

Relative paths inside the crate work as well:

    #[doc = rsdoc::image!(images/schema.png)]

#### Drawio

`image!` works with drawio's *editable vector image* format. In drawio pick
`File > Export as > SVG` and tick **Include a copy of my diagram**; the suggested
file name then ends in `.drawio.svg`.

That single file plays both roles: browsers render it as an ordinary vector
image, and drawio reopens it as the original diagram, because the source XML is
stored in the SVG element's `content` attribute. There is no separate `.drawio`
file to keep in sync.

    #[doc = rsdoc::image!(diagrams/architecture.drawio.svg)]

rsdoc does not re-render `.drawio` sources — export again after changing a
diagram. To check that a file really carries its source:

```sh
grep -c "mxfile\|mxGraphModel" diagrams/architecture.drawio.svg
```

A result of `0` means the export was a plain image, so keep the `.drawio` file
alongside it.

### Tips

 - These macros expand during **every** build, not only `cargo doc`, because
   `#[doc = ...]` is a regular attribute. Rendered diagrams are therefore cached
   in `target/rsdoc-cache/<sha1>.svg`; a rebuild reads the cache instead of
   rendering again. Delete that directory to force a refresh.
 - Wrap a call in `#[cfg_attr(doc, ...)]` if you only want the diagram in
   documentation builds.
 - The bundled `mermaid.js` adds roughly 3.4 MiB to the crate. It is written to
   the documentation root once per build tree, not embedded into every page.

### Mermaid syntax check

Mermaid is drawn by the browser, so a syntax error would otherwise stay invisible
until someone opens the documentation. rsdoc runs a lightweight structural check
during the build and reports what it finds:

```text
warning: rsdoc failed on `grap TD A --> B` (mermaid):
         unrecognised diagram type `grap`, did you mean `graph`?
warning: rsdoc failed on `graph A --> B` (mermaid):
         `graph` needs a direction (TD, TB, BT, LR or RL), found ``
warning: rsdoc failed on `graph TD A[Start --> B[Done]` (mermaid):
         line 2: `[` is never closed
```

It covers a misspelled or unsupported diagram type, a `graph` with no direction,
unbalanced brackets or quotes, and a `subgraph` without `end`. Problems are
reported but never block rendering.

This is deliberately not a full parser — it will not tell you that a layout looks
wrong, only that the source cannot be right.

### For scripts and AI agents

Set `RSDOC_JSON=1` and every diagnostic becomes one JSON object per line instead
of prose:

```json
{"tool":"rsdoc","level":"error","kind":"plantuml","message":"syntax error at line 3: Syntax Error?","preview":"@startuml Alice -> Bob : hello ThisLineIsNotVali…"}
```

Every failure also names the diagram that broke. A `#[doc]` expansion carries no
usable source location on stable Rust, so in a crate with dozens of diagrams the
source excerpt is the only way to tell which one failed.

### How asset paths are resolved

rsdoc deliberately does **not** hard-code a relative prefix such as
`../../images/`. The depth of a rustdoc page depends on where the item sits in
the module tree:

| page | path back to the doc root |
| --- | --- |
| `crate/fn.x.html` | `../` |
| `crate/mod/fn.x.html` | `../../` |
| `crate/a/b/fn.x.html` | `../../../` |

Any single fixed value is therefore wrong for some pages. Instead, every
generated element carries a `data-rsdoc-src` attribute and a small bootstrap
script fills in the real `src` at load time. The root it prepends is derived from
rustdoc's own asset reference, which rustdoc already computes correctly per page:

1. `static.files/` — rustdoc 1.65 and later
2. `rustdoc.css` at the doc root — older releases
3. the `Source` link, which always points at `<root>/src/…`

If none of them can be found — for instance if a future rustdoc switches to
absolute asset URLs — the bootstrap falls back to probing `../`, `../../`,
`../../../` and `./` until one resolves. A working anchor costs exactly one
request, so probing only fires when the page structure has really changed.

The output directory itself is located from `CARGO_TARGET_DIR`, or by walking
upwards from `CARGO_MANIFEST_DIR` until a `target/` directory appears, so
workspaces and custom target directories work too.

License: MIT OR Apache-2.0
