use facet::Facet;
use facet_value::Value;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Facet)]
pub struct Heading {
    pub title: String,
    pub id: String,
    pub level: u8,
}

#[derive(Debug, Clone, Default, Facet)]
pub struct Frontmatter {
    pub title: String,
    pub weight: i32,
    pub description: Option<String>,
    pub template: Option<String>,
    pub extra: Value,
}

/// A file shipped alongside the main document so the cell can resolve
/// `include::target[]` directives without reading the host filesystem.
/// `path` is content-dir-relative with `/` separators.
#[derive(Debug, Clone, Facet)]
pub struct IncludeFile {
    pub path: String,
    pub content: String,
}

// ============================================================================
// Include target resolution
//
// Shared by the host (to decide which tracked files to ship and register as
// build dependencies) and the cell (to refuse documents whose include
// targets cannot be proven to stay inside the content directory). Keeping
// both sides on one implementation means they can never disagree about
// which targets are safe.
// ============================================================================

/// Extract raw `include::target[attrs]` targets from AsciiDoc source.
/// Only unescaped directives at the start of a line count, matching the
/// AsciiDoc preprocessor (a leading `\` escapes the directive).
pub fn scan_include_targets(content: &str) -> Vec<&str> {
    content
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("include::")?;
            let (target, attrs) = rest.split_once('[')?;
            attrs.trim_end().ends_with(']').then_some(target)
        })
        .collect()
}

/// Resolve an include target against the content-dir-relative path of the
/// file containing the directive. Returns the normalized content-dir-relative
/// path of the target, or `None` when the target cannot be statically proven
/// to stay inside the content directory: absolute paths, `..` escapes past
/// the content root, URLs / Windows drive letters (`:`), backslashes, and
/// attribute references (`{attr}`) are all rejected.
pub fn resolve_include_target(includer_path: &str, target: &str) -> Option<String> {
    if target.is_empty()
        || target.starts_with('/')
        || target.contains(':')
        || target.contains('\\')
        || target.contains('{')
    {
        return None;
    }
    let mut parts: Vec<&str> = match includer_path.rsplit_once('/') {
        Some((dir, _)) => dir.split('/').collect(),
        None => Vec::new(),
    };
    for seg in target.split('/') {
        match seg {
            "" => return None,
            "." => {}
            ".." => {
                parts.pop()?;
            }
            seg => parts.push(seg),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

// ============================================================================
// Result types
// ============================================================================

#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum ParseResult {
    Success {
        frontmatter: Frontmatter,
        html: String,
        headings: Vec<Heading>,
        head_injections: Vec<String>,
    },
    Error {
        message: String,
    },
}

// ============================================================================
// Cell service
// ============================================================================

#[allow(async_fn_in_trait)]
#[vox::service]
pub trait AsciiDocProcessor {
    async fn parse_and_render(
        &self,
        source_path: String,
        content: String,
        includes: Vec<IncludeFile>,
    ) -> ParseResult;
}

#[cfg(test)]
mod tests {
    use super::*;
    use facet_value::{DestructuredRef, VObject, VString};

    #[test]
    fn frontmatter_extra_roundtrip() {
        let mut extra = VObject::new();
        extra.insert(VString::from("sidebar"), Value::from(true));
        extra.insert(VString::from("icon"), Value::from("book"));

        let fm = Frontmatter {
            title: "Test".to_string(),
            weight: 5,
            description: Some("A test".to_string()),
            template: None,
            extra: Value::from(extra),
        };

        let bytes = facet_postcard::to_vec(&fm).expect("serialize");
        let fm2: Frontmatter = facet_postcard::from_slice(&bytes).expect("deserialize");

        assert_eq!(fm2.title, "Test");
        assert_eq!(fm2.weight, 5);
        assert_eq!(fm2.description.as_deref(), Some("A test"));
        match fm2.extra.destructure_ref() {
            DestructuredRef::Object(obj) => {
                let sidebar = obj.get("sidebar").expect("sidebar");
                assert_eq!(sidebar.as_bool(), Some(true));
                let icon = obj.get("icon").expect("icon");
                assert_eq!(icon.as_string().unwrap().as_str(), "book");
            }
            other => panic!("expected object, got {:?}", other),
        }
    }

    #[test]
    fn scan_finds_line_leading_directives_only() {
        let content = "= Doc\n\ninclude::partials/_a.adoc[]\n text include::no.adoc[]\n\\include::escaped.adoc[]\ninclude::snippets/code.adoc[lines=1..3]\ninclude::unterminated.adoc\n";
        assert_eq!(
            scan_include_targets(content),
            vec!["partials/_a.adoc", "snippets/code.adoc"]
        );
    }

    #[test]
    fn resolve_relative_to_includer_dir() {
        assert_eq!(
            resolve_include_target("posts/git.adoc", "_parts/setup.adoc"),
            Some("posts/_parts/setup.adoc".to_string())
        );
        assert_eq!(
            resolve_include_target("page.adoc", "shared.adoc"),
            Some("shared.adoc".to_string())
        );
        assert_eq!(
            resolve_include_target("a/b/c.adoc", "../sibling.adoc"),
            Some("a/sibling.adoc".to_string())
        );
        assert_eq!(
            resolve_include_target("a/b/c.adoc", "./d.adoc"),
            Some("a/b/d.adoc".to_string())
        );
    }

    #[test]
    fn resolve_rejects_escapes_and_dynamic_targets() {
        // absolute path
        assert_eq!(resolve_include_target("posts/p.adoc", "/etc/passwd"), None);
        // escape past content root
        assert_eq!(resolve_include_target("posts/p.adoc", "../../x.adoc"), None);
        assert_eq!(resolve_include_target("p.adoc", "../x.adoc"), None);
        // URLs and Windows drive letters
        assert_eq!(
            resolve_include_target("p.adoc", "https://example.com/x.adoc"),
            None
        );
        assert_eq!(resolve_include_target("p.adoc", "C:\\x.adoc"), None);
        // attribute references cannot be statically verified
        assert_eq!(resolve_include_target("p.adoc", "{partialsdir}/x.adoc"), None);
        // empty / directory-only targets
        assert_eq!(resolve_include_target("p.adoc", ""), None);
        assert_eq!(resolve_include_target("posts/p.adoc", ".."), None);
    }
}
