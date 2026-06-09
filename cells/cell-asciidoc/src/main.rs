use acdc_converters_core::Options;
use acdc_converters_html::{HtmlVariant, Processor, RenderOptions};
use acdc_parser::inlines_to_string;
use cell_asciidoc_proto::*;
use dodeca_cell_runtime::tracing;
use facet_value::{VObject, VString, Value};
use std::time::Instant;

#[derive(Clone)]
pub struct AsciiDocProcessorImpl;

impl AsciiDocProcessor for AsciiDocProcessorImpl {
    async fn parse_and_render(&self, source_path: String, content: String) -> ParseResult {
        let started_at = Instant::now();
        tracing::debug!(
            source_path = %source_path,
            content_len = content.len(),
            "asciidoc cell parse_and_render started"
        );

        let parse_opts = acdc_parser::Options::default();
        let parsed = match acdc_parser::parse(&content, &parse_opts) {
            Ok(p) => p,
            Err(e) => {
                return ParseResult::Error {
                    message: e.to_string(),
                };
            }
        };
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
        let known = ["weight", "description", "template", "doctitle", "toc", "sectnums"];
        for (name, value) in doc.attributes.iter() {
            let n = name.as_ref();
            if known.contains(&n) {
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
            HtmlVariant::Standard,
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
            head_injections: Vec::new(),
        }
    }
}

dodeca_cell_runtime::declare_cell!("asciidoc", |_host| {
    AsciiDocProcessorDispatcher::new(AsciiDocProcessorImpl)
});
