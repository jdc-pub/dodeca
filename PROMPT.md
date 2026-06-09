# AsciiDoc support in dodeca

Add `.adoc` rendering alongside Markdown. Parse with `acdc-parser` (crates.io 0.9.0),
render HTML with `acdc-converters-html` + `acdc-converters-core` (git deps, pin a rev).
See `../acdx/crates/acdx/src/adoc.rs` for the AST (`Document`/`Block`) usage pattern.
Parallels `cell-markdown`; Markdown path stays untouched.

## Decisions
- Extensions: `.adoc` only; section index `_index.adoc`.
- Metadata: doc header `= Title` → `title`; attributes → `weight`/`description`/`template`, rest → `extra`. No TOML frontmatter.
- v1: use acdc's own listing/highlight output; `reqs` and `source_map` empty. Dodeca code-block handlers (arborium/mermaid/…) deferred.

## Work
1. **`cells/cell-asciidoc-proto`** — mirror `cell-markdown-proto` subset: `Heading{title,id,level}`, `Frontmatter{title,weight,description,template,extra}`, `enum ParseResult{Success{frontmatter,html,headings,head_injections},Error{message}}`, trait `AsciiDocProcessor::parse_and_render(source_path, content) -> ParseResult`.
2. **`cells/cell-asciidoc`** (bin `ddc-cell-asciidoc`) — `acdc_parser::parse` → map header/attrs to `Frontmatter`, walk `Block::Section` for headings, `Processor::convert_to_string(&doc, RenderOptions{embedded:true,..})`. Wire with `cell_service!` + `run_cell!("asciidoc", …)`.
3. **Register** — `cells.rs`: `cell_client_accessor!(asciidoc_cell,"asciidoc",AsciiDocProcessorClient)`, `parse_and_render_asciidoc_cell` (mirror `cells.rs:646`), `CellDef::new("asciidoc")`. `host.rs`: `impl_cell_client!(…AsciiDocProcessorClient,"asciidoc")`. Add path dep to `crates/dodeca/Cargo.toml` (+ root members if not globbed).
4. **De-hardcode `.md`** — `build_context.rs:135` filter `md|adoc`; `types.rs` `is_section_index`/`to_route` handle `.adoc`, add `is_asciidoc()`; `queries.rs:333` `parse_file` dispatch on ext to asciidoc cell, fill empty `reqs`/`source_map`. Rest of `parse_file` unchanged (shapes match).
5. **CI** — add `ddc-cell-asciidoc` to plugin list in `xtask/src/ci.rs`, run `cargo xtask ci`.

## Verify
`cargo xtask build`; add `content/asciidoc-test.adoc` (header, headings, list, `[source,rust]`); `cargo xtask run -- serve --no-tui` → load `/asciidoc-test` (title/TOC/body/chrome render, `.md` unaffected); `cargo xtask run -- build`; run tests.
