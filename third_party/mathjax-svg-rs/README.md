# mathjax-svg-rs

Very thin wrapper around MathJax to render TeX to SVG, using QuickJS-ng as the JavaScript engine.

## Usage

```rust
let svg = mathjax_svg_rs::render_tex(&tex).expect("Failed to render TeX");
```

```rust
let svg = mathjax_svg_rs::render_tex_with_font_size(&tex, 32.0).expect("Failed to render TeX");
```

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
