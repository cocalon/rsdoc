//! Plain images, including drawio exports.
//!
//! ```sh
//! cargo doc --example images --open
//! ```

/// Any picture works — PNG, JPEG or SVG.
#[doc = rsdoc::image!(examples/assets/pixel.png)]
pub fn png() {}

/// Drawio's *editable vector image* format keeps the source XML inside the SVG's
/// `content` attribute, so one `.drawio.svg` file is both the picture and the
/// editable diagram:
///
/// ```sh
/// # in drawio: File > Export as > SVG, tick "Include a copy of my diagram"
/// ```
#[doc = rsdoc::image!(examples/assets/pixel.png)]
pub fn drawio_style() {}

fn main() {}
