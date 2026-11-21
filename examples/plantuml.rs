//! PlantUML diagrams.
//!
//! Rendering needs either a local `plantuml.jar` — found through the
//! `RSDOC_PLANTUML_JAR` environment variable, next to `Cargo.toml`, or in the
//! workspace root — or network access to `plantuml.com`. Results are cached under
//! `target/rsdoc-cache/` and keyed on the jar, so a rebuild reads the cache
//! instead of asking again. Set `RSDOC_PLANTUML_OFFLINE=1` to forbid the network.
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
