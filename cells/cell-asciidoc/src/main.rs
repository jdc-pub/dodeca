use acdc_converters_core::Options;
use acdc_converters_html::{HtmlVariant, Processor, RenderOptions};
use acdc_parser::inlines_to_string;
use cell_asciidoc_proto::*;
use dodeca_cell_runtime::tracing;
use facet_value::{VObject, VString, Value};
use std::time::Instant;

#[derive(Clone)]
pub struct AsciiDocProcessorImpl;

/// Reject documents whose `include::` targets cannot be statically proven to
/// stay inside the content directory. acdc's preprocessor resolves include
/// paths eagerly with no safe-mode enforcement for file targets, so this
/// check must run before any parsing touches the filesystem.
fn unsafe_include_error(
    source_path: &str,
    content: &str,
    includes: &[IncludeFile],
) -> Option<String> {
    if resolve_include_target("", source_path).as_deref() != Some(source_path) {
        return Some(format!(
            "invalid source path {source_path:?}: must be a normalized relative path"
        ));
    }
    let files = std::iter::once((source_path, content))
        .chain(includes.iter().map(|f| (f.path.as_str(), f.content.as_str())));
    for (path, text) in files {
        for target in scan_include_targets(text) {
            if resolve_include_target(path, target).is_none() {
                return Some(format!(
                    "unsafe include target {target:?} in {path}: include targets must be \
                     relative paths that stay inside the content directory"
                ));
            }
        }
    }
    None
}

/// Materialize the document and its includes into a temp directory mirroring
/// their content-dir-relative layout, then parse from there. The temp dir is
/// the only filesystem the parse can see legitimate targets in; combined with
/// `unsafe_include_error` this keeps include resolution hermetic.
fn parse_in_tempdir(
    source_path: &str,
    content: &str,
    includes: &[IncludeFile],
    opts: &acdc_parser::Options,
) -> Result<acdc_parser::ParseResult, String> {
    let dir = tempfile::tempdir()
        .map_err(|e| format!("failed to create temp dir for include resolution: {e}"))?;
    let files = std::iter::once((source_path, content))
        .chain(includes.iter().map(|f| (f.path.as_str(), f.content.as_str())));
    for (path, text) in files {
        let abs = dir.path().join(path);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
        }
        std::fs::write(&abs, text).map_err(|e| format!("failed to write {path}: {e}"))?;
    }
    acdc_parser::parse_file(dir.path().join(source_path), opts).map_err(|e| e.to_string())
}

impl AsciiDocProcessor for AsciiDocProcessorImpl {
    async fn parse_and_render(
        &self,
        source_path: String,
        content: String,
        includes: Vec<IncludeFile>,
    ) -> ParseResult {
        let started_at = Instant::now();
        tracing::debug!(
            source_path = %source_path,
            content_len = content.len(),
            include_count = includes.len(),
            "asciidoc cell parse_and_render started"
        );

        if let Some(message) = unsafe_include_error(&source_path, &content, &includes) {
            return ParseResult::Error { message };
        }

        let parse_opts = acdc_parser::Options::default();
        let has_includes = !scan_include_targets(&content).is_empty()
            || includes.iter().any(|f| !scan_include_targets(&f.content).is_empty());
        let parsed = if has_includes {
            match parse_in_tempdir(&source_path, &content, &includes, &parse_opts) {
                Ok(p) => p,
                Err(message) => return ParseResult::Error { message },
            }
        } else {
            match acdc_parser::parse(&content, &parse_opts) {
                Ok(p) => p,
                Err(e) => {
                    return ParseResult::Error {
                        message: e.to_string(),
                    };
                }
            }
        };
        for warning in parsed.warnings() {
            tracing::warn!(source_path = %source_path, "asciidoc: {warning}");
        }
        let doc = parsed.document();

        // Extract title from header
        let title = doc
            .header
            .as_ref()
            .map(|h| inlines_to_string(&h.title))
            .unwrap_or_default();

        // Extract known scalar attributes; collect the rest as `extra`
        let weight = doc
            .attributes
            .get_string("weight")
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(0);
        let description = doc
            .attributes
            .get_string("description")
            .map(|s| s.into_owned());
        let template = doc
            .attributes
            .get_string("template")
            .map(|s| s.into_owned());

        let mut extra = VObject::new();
        // Scalar fields extracted above; skip them here.
        let top_level_fields = ["weight", "template"];
        // AsciiDoc predefined/built-in attributes from acdc-parser constants.rs.
        // These must be filtered out of `extra` so only user-defined metadata appears.
        const BUILTIN_ATTRS: &[&str] = &[
            // Character replacements / intrinsic
            "empty", "blank", "sp", "nbsp", "zwsp", "wj", "apos", "quot", "lsquo", "rsquo",
            "ldquo", "rdquo", "deg", "plus", "brvbar", "vbar", "amp", "lt", "gt", "startsb",
            "endsb", "caret", "asterisk", "tilde", "backslash", "backtick", "two-colons",
            "two-semicolons", "cpp", "cxx", "pp",
            // Admonition captions
            "note-caption", "tip-caption", "important-caption", "caution-caption",
            "warning-caption",
            // Block captions
            "example-caption", "figure-caption", "table-caption", "appendix-caption",
            // Reference labels
            "section-refsig", "chapter-refsig", "part-refsig", "appendix-refsig",
            // UI labels
            "toc-title", "version-label", "untitled-label", "last-update-label",
            // Structural settings
            "idprefix", "idseparator", "sectids", "sectnumlevels", "toclevels",
            "toc", "sectnums",
            // Attribute processing
            "attribute-undefined", "attribute-missing",
            // Author-derived (set by acdc from the :author: line)
            "firstname", "lastname", "authorinitials", "authors", "authorcount",
            // Document internals
            "doctitle",
        ];
        for (name, value) in doc.attributes.iter() {
            let n = name.as_ref();
            if top_level_fields.contains(&n) || BUILTIN_ATTRS.contains(&n) {
                continue;
            }
            let v = match value {
                acdc_parser::AttributeValue::String(s) => Value::from(s.as_ref()),
                acdc_parser::AttributeValue::Bool(b) => Value::from(*b),
                acdc_parser::AttributeValue::None => Value::from(true),
                _ => continue,
            };
            extra.insert(VString::from(n), v);
        }

        let frontmatter = Frontmatter {
            title,
            weight,
            description,
            template,
            extra: Value::from(extra),
        };

        // Extract headings from toc_entries
        let headings: Vec<Heading> = doc
            .toc_entries
            .iter()
            .map(|e| Heading {
                title: inlines_to_string(&e.title),
                id: e.id.to_string(),
                level: e.level,
            })
            .collect();

        // Render HTML (embedded = no DOCTYPE/html/head/body wrapper)
        let converter_opts = Options::builder().embedded(true).build();
        let processor = Processor::new_with_variant(
            converter_opts,
            doc.attributes.clone(),
            HtmlVariant::Semantic,
        );
        let render_opts = RenderOptions {
            embedded: true,
            ..RenderOptions::default()
        };
        let html = match processor.convert_to_string(doc, &render_opts) {
            Ok(h) => h,
            Err(e) => {
                return ParseResult::Error {
                    message: e.to_string(),
                };
            }
        };

        tracing::debug!(
            elapsed_ms = started_at.elapsed().as_millis(),
            html_len = html.len(),
            heading_count = headings.len(),
            "asciidoc cell parse_and_render finished"
        );

        ParseResult::Success {
            frontmatter,
            html,
            headings,
            head_injections: vec![
                // AsciiDoc wraps list item content in <p> tags, adding unwanted
                // bottom margin from site CSS `p { margin-bottom: ... }` rules.
                r#"<style>li > p { margin-bottom: 0; }</style>"#.to_string(),
            ],
        }
    }
}

dodeca_cell_runtime::declare_cell!("asciidoc", |_host| {
    AsciiDocProcessorDispatcher::new(AsciiDocProcessorImpl)
});

#[cfg(test)]
mod tests {
    use super::*;

    async fn render(content: &str) -> ParseResult {
        AsciiDocProcessorImpl
            .parse_and_render("test.adoc".to_string(), content.to_string(), Vec::new())
            .await
    }

    async fn render_at(source_path: &str, content: &str, includes: Vec<IncludeFile>) -> ParseResult {
        AsciiDocProcessorImpl
            .parse_and_render(source_path.to_string(), content.to_string(), includes)
            .await
    }

    #[tokio::test]
    async fn basic_render_produces_html() {
        let result = render("= Hello\n\nSome text.\n").await;
        let ParseResult::Success { html, frontmatter, .. } = result else {
            panic!("expected Success");
        };
        assert_eq!(frontmatter.title, "Hello");
        assert!(html.contains("Some text."), "html={html}");
    }

    #[tokio::test]
    async fn embedded_html_has_no_doctype() {
        let result = render("= Doc\n\nBody.\n").await;
        let ParseResult::Success { html, .. } = result else {
            panic!("expected Success");
        };
        assert!(!html.contains("<!DOCTYPE"), "should be embedded, not full page: {html}");
        assert!(!html.contains("<html"), "should be embedded, not full page: {html}");
    }

    #[tokio::test]
    async fn description_is_in_extra() {
        let content = "= My Page\n:description: A desc\n:date: 2026-01-01\n\nContent.\n";
        let result = render(content).await;
        let ParseResult::Success { frontmatter, .. } = result else {
            panic!("expected Success");
        };
        use facet_value::DestructuredRef;
        // description must be in extra so templates can access page.extra.description
        let DestructuredRef::Object(obj) = frontmatter.extra.destructure_ref() else {
            panic!("extra must be an object");
        };
        assert!(
            obj.get("description").is_some(),
            "description must be in extra"
        );
        // also still available as the top-level field for OG meta etc.
        assert_eq!(frontmatter.description.as_deref(), Some("A desc"));
    }

    #[tokio::test]
    async fn builtin_attrs_not_in_extra() {
        let content = "= My Page\n:description: A desc\n:date: 2026-01-01\n:author: Jane Doe\n\nContent.\n";
        let result = render(content).await;
        let ParseResult::Success { frontmatter, .. } = result else {
            panic!("expected Success");
        };
        use facet_value::DestructuredRef;
        let DestructuredRef::Object(obj) = frontmatter.extra.destructure_ref() else {
            panic!("extra must be an object");
        };
        // user-defined attrs must appear
        assert!(obj.get("description").is_some(), "description missing");
        assert!(obj.get("date").is_some(), "date missing");
        assert!(obj.get("author").is_some(), "author missing");
        // acdc-derived and built-in attrs must not appear
        for builtin in &["note-caption", "toc-title", "firstname", "lastname", "authorinitials", "blank", "sp"] {
            assert!(
                obj.get(*builtin).is_none(),
                "builtin attr {builtin:?} should not be in extra"
            );
        }
    }

    #[tokio::test]
    async fn frontmatter_attributes_extracted() {
        let content = "= My Page\n:weight: 7\n:description: A desc\n\nContent.\n";
        let result = render(content).await;
        let ParseResult::Success { frontmatter, .. } = result else {
            panic!("expected Success");
        };
        assert_eq!(frontmatter.title, "My Page");
        assert_eq!(frontmatter.weight, 7);
        assert_eq!(frontmatter.description.as_deref(), Some("A desc"));
    }

#[tokio::test]
    async fn headings_extracted() {
        let content = "= Top\n\n== Introduction\n\nText.\n\n== Conclusion\n\nEnd.\n";
        let result = render(content).await;
        let ParseResult::Success { headings, .. } = result else {
            panic!("expected Success");
        };
        assert_eq!(headings.len(), 2);
        assert_eq!(headings[0].title, "Introduction");
        assert_eq!(headings[0].level, 1);
        assert_eq!(headings[1].title, "Conclusion");
        assert_eq!(headings[1].level, 1);
    }

    #[tokio::test]
    async fn nested_headings_have_correct_levels() {
        let content = "= Doc\n\n== Section\n\n=== Subsection\n\nText.\n";
        let result = render(content).await;
        let ParseResult::Success { headings, .. } = result else {
            panic!("expected Success");
        };
        // acdc uses 1-based levels relative to the doc title: == is level 1, === is level 2
        let levels: Vec<u8> = headings.iter().map(|h| h.level).collect();
        assert!(levels.contains(&1), "expected level 1 heading (==): {levels:?}");
        assert!(levels.contains(&2), "expected level 2 heading (===): {levels:?}");
    }

    // AsciiDoc wraps list item content in <p> tags:
    //   <li><p>text</p></li>
    // Sites built for markdown expect bare <li>text</li>. Without normalisation,
    // CSS like `p { margin-bottom: 1.25rem }` adds unwanted spacing inside each
    // list item. head_injections must include a CSS rule to zero that out.
    #[tokio::test]
    async fn head_injections_normalize_li_paragraph_margin() {
        let content = "= Doc\n\n. First item\n. Second item\n";
        let result = render(content).await;
        let ParseResult::Success { head_injections, .. } = result else {
            panic!("expected Success");
        };
        let combined = head_injections.join("\n");
        assert!(
            combined.contains("li") && combined.contains("margin-bottom"),
            "head_injections must normalize li > p margin; got: {combined:?}"
        );
    }

    #[tokio::test]
    async fn include_directive_inlines_supplied_partial() {
        let content = "= Doc\n\ninclude::_parts/body.adoc[]\n";
        let includes = vec![IncludeFile {
            path: "_parts/body.adoc".to_string(),
            content: "Included paragraph from partial.".to_string(),
        }];
        let result = render_at("posts/page.adoc", content, includes).await;
        let ParseResult::Success { html, .. } = result else {
            panic!("expected Success, got {result:?}");
        };
        assert!(
            html.contains("Included paragraph from partial."),
            "partial content must be inlined; got: {html}"
        );
    }

    #[tokio::test]
    async fn nested_include_relative_to_partial_dir() {
        // page includes _parts/a.adoc, which itself includes b.adoc (a sibling).
        let content = "= Doc\n\ninclude::_parts/a.adoc[]\n";
        let includes = vec![
            IncludeFile {
                path: "_parts/a.adoc".to_string(),
                content: "From A.\n\ninclude::b.adoc[]\n".to_string(),
            },
            IncludeFile {
                path: "_parts/b.adoc".to_string(),
                content: "From B.".to_string(),
            },
        ];
        let result = render_at("page.adoc", content, includes).await;
        let ParseResult::Success { html, .. } = result else {
            panic!("expected Success, got {result:?}");
        };
        assert!(html.contains("From A."), "A missing: {html}");
        assert!(html.contains("From B."), "nested include B missing: {html}");
    }

    #[tokio::test]
    async fn unsafe_include_target_is_rejected() {
        let content = "= Doc\n\ninclude::../../../etc/passwd[]\n";
        let result = render_at("posts/page.adoc", content, Vec::new()).await;
        let ParseResult::Error { message } = result else {
            panic!("expected Error for path-escaping include, got {result:?}");
        };
        assert!(
            message.contains("unsafe include target"),
            "error must name the unsafe target; got: {message}"
        );
    }

    #[tokio::test]
    async fn no_includes_skips_tempdir_path() {
        // A document with no include directives must still render via the
        // in-memory path (regression guard on the has_includes branch).
        let result = render_at("page.adoc", "= Doc\n\nPlain body.\n", Vec::new()).await;
        let ParseResult::Success { html, .. } = result else {
            panic!("expected Success");
        };
        assert!(html.contains("Plain body."), "got: {html}");
    }
}
