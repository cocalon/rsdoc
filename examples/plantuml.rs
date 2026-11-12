//! PlantUML diagrams.
//!
//! Rendering needs either a local `plantuml.jar` — found through the
//! `RSDOC_PLANTUML_JAR` environment variable or next to `Cargo.toml` — or network
//! access to `plantuml.com`. Results are cached under `target/rsdoc-cache/`, so a
//! rebuild reads the cache instead of asking again.
//!
//! ```sh
//! cargo doc --example plantuml --open
//! ```

/// Attribute form: a fenced `plantuml` block. The `@startuml` / `@enduml`
/// wrappers are optional here and are added when missing.
///
/// ```plantuml
/// @startuml
/// !theme cyborg-outline
/// Bob -> Alice : hello
/// Alice --> Bob : hi
/// @enduml
/// ```
#[rsdoc::diagram]
pub fn attribute_style() {}

/// Function-style macro, kept for compatibility.
#[doc = rsdoc::plantuml!(
/// @startuml
/// Alice -> Bob : another one
/// @enduml
)]
pub fn function_style() {}

fn main() {}
