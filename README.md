# rsdoc

This crate provides a procedural macro that transform
PlandUML and Drawio diagrams in doc comments as PNG or SVG images.
The diagrams in doc comments as SVG images using [`drawio`].
The UML diagrams and flow diagrams in doc comments as PNG images using [`plantUML`].

*Requires Rust version 1.54 or later or equivalent nightly builds.*

[`drawio`]: https://drawio-app.com/
[`plantuml`]: https://www.plantuml.com/
          https://raw.githubusercontent.com/cocalon/rsdoc/main/rsdoc_example.png
<img src="https://raw.githubusercontent.com/cocalon/rsdoc/main/rsdoc_example.png"
   style="border: 10px solid rgba(192, 192, 192, 0.15)">

## Usage

Add the following line to `Cargo.toml`.

```toml
[dependencies]
rsdoc = "0.1.0"
```

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

### `plantuml_file!`
Or you can directly attach the puml file stored as a file
    
    #[doc = rsdoc::plantuml_file!(test.puml)]

If you want to attach pictures, such as PNG or SVG, you can do it this way   

    #[doc = rsdoc::image!(test.png)]

### `image!`
And you can also use it with the Drawio tool, hope you like it!

    #[doc = rsdoc::image!(test.drawio.svg)]


See the `example` directory for a complete example.

### Tips

 - Using this macro increases the compilation time. The `enable` Cargo feature can be used to turn off the transformation and the compilation of most dependent packages.

License: MIT/Apache-2.0
