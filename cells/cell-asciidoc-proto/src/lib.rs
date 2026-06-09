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
    async fn parse_and_render(&self, source_path: String, content: String) -> ParseResult;
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
}
