//! Mermaid diagrams — no network required.
//!
//! The bundled `mermaid.js` is copied into the documentation root at build time
//! and the diagram is drawn by the browser, so this example works fully offline.
//!
//! ```sh
//! cargo doc --example mermaid --open
//! ```

/// Preferred form: a fenced `mermaid` block inside the doc comment.
///
/// Surrounding prose is preserved, both before and after the block.
///
/// ```mermaid
/// graph TD
///   A[rustdoc] --> B{Which syntax?}
///   B -->|mermaid| C[Drawn in the browser, offline]
///   B -->|plantuml| D[Rendered by plantuml.com]
/// ```
#[rsdoc::diagram]
pub fn attribute_style() {}

/// The function-style macro, kept for compatibility.
#[doc = rsdoc::mermaid!(
/// sequenceDiagram
///   Alice->>Bob: Ping
///   Bob-->>Alice: Pong
)]
pub fn function_style() {}

/// Load a larger diagram from its own `.mmd` file.
#[doc = rsdoc::mermaid_file!("examples/assets/flow.mmd")]
pub fn from_file() {}

fn main() {}
