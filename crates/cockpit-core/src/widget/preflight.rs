use std::io::{self, Write};

use cockpit_protocol::widget::WIDGET_MAX_HTML_BYTES;
use html5ever::{parse_document, serialize, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};

use crate::InspectionError;

use super::text;

const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 256;

#[derive(Debug)]
pub struct Preflight {
    pub document: String,
    pub title: Option<String>,
    pub warnings: Vec<String>,
}

pub fn sanitize(input: &str) -> Result<Preflight, InspectionError> {
    if input.len() > WIDGET_MAX_HTML_BYTES {
        return Err(InspectionError::new(
            "widget_too_large",
            "HTML exceeds 1 MiB",
        ));
    }
    let dom = parse_document(RcDom::default(), Default::default()).one(input);
    // Inspect the original tree before pruning: removed nodes still count
    // toward complexity.
    let title = inspect(&dom.document)?;
    let mut removed_base = false;
    let mut removed_meta_policy = false;
    let mut pending = vec![dom.document.clone()];
    while let Some(node) = pending.pop() {
        node.children.borrow_mut().retain(|child| {
            let NodeData::Element { name, attrs, .. } = &child.data else {
                return true;
            };
            if name.local.as_ref().eq_ignore_ascii_case("base") {
                removed_base = true;
                return false;
            }
            if name.local.as_ref().eq_ignore_ascii_case("meta")
                && attrs
                    .borrow()
                    .iter()
                    .any(|attr| attr.name.local.as_ref().eq_ignore_ascii_case("http-equiv"))
            {
                removed_meta_policy = true;
                return false;
            }
            true
        });
        pending.extend(node.children.borrow().iter().rev().cloned());
        if let NodeData::Element {
            template_contents, ..
        } = &node.data
        {
            if let Some(contents) = template_contents.borrow().as_ref() {
                pending.push(contents.clone());
            }
        }
    }
    // Fixed, deduplicated diagnostics never echo arbitrary author text into
    // terminal output or Cockpit chrome.
    let mut warnings = Vec::new();
    if removed_base {
        warnings.push("host_policy_preserved: base elements removed; Cockpit supplies the document base policy".to_owned());
    }
    if removed_meta_policy {
        warnings.push(
            "host_policy_preserved: meta[http-equiv] removed; Cockpit supplies the document policy"
                .to_owned(),
        );
    }
    let mut output = BoundedDocument {
        bytes: Vec::with_capacity(input.len()),
    };
    serialize(
        &mut output,
        &SerializableHandle::from(dom.document.clone()),
        Default::default(),
    )
    .map_err(|_| InspectionError::new("widget_too_large", "rendered HTML exceeds 1 MiB"))?;
    let document = String::from_utf8(output.bytes).map_err(|_| {
        InspectionError::new("widget_usage", "HTML could not be serialized as UTF-8")
    })?;
    Ok(Preflight {
        document,
        title,
        warnings,
    })
}

struct BoundedDocument {
    bytes: Vec<u8>,
}

impl Write for BoundedDocument {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > WIDGET_MAX_HTML_BYTES - self.bytes.len() {
            return Err(io::Error::other("rendered HTML exceeds 1 MiB"));
        }
        // Vec's geometric growth must not allocate beyond the content cap.
        if self.bytes.capacity() - self.bytes.len() < bytes.len() {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(self.bytes.len() + bytes.len())
                .min(WIDGET_MAX_HTML_BYTES);
            self.bytes.reserve_exact(capacity - self.bytes.len());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn inspect(root: &Handle) -> Result<Option<String>, InspectionError> {
    let mut pending = vec![(root.clone(), 0)];
    let mut nodes = 0;
    let mut title = None;
    while let Some((node, depth)) = pending.pop() {
        nodes += 1;
        if nodes > MAX_NODES || depth > MAX_DEPTH {
            return Err(InspectionError::new(
                "widget_too_complex",
                "HTML exceeds 100000 nodes or depth 256",
            ));
        }
        if let NodeData::Element {
            name,
            template_contents,
            ..
        } = &node.data
        {
            if title.is_none()
                && name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                && name.local.as_ref().eq_ignore_ascii_case("title")
            {
                let mut raw = String::new();
                for child in node.children.borrow().iter() {
                    if let NodeData::Text { contents } = &child.data {
                        raw.push_str(contents.borrow().as_ref());
                    }
                }
                let sanitized = text::sanitize(&raw, 80);
                if !sanitized.is_empty() {
                    title = Some(sanitized);
                }
            }
            if let Some(contents) = template_contents.borrow().as_ref() {
                pending.push((contents.clone(), depth + 1));
            }
        }
        pending.extend(
            node.children
                .borrow()
                .iter()
                .rev()
                .map(|child| (child.clone(), depth + 1)),
        );
    }
    Ok(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elements(input: &str) -> Vec<(String, Vec<String>)> {
        let dom = parse_document(RcDom::default(), Default::default()).one(input);
        // RcDom's node destructor clears descendants even if handles remain queued.
        let mut pending = vec![dom.document.clone()];
        let mut result = Vec::new();
        while let Some(node) = pending.pop() {
            pending.extend(node.children.borrow().iter().cloned());
            if let NodeData::Element { name, attrs, .. } = &node.data {
                result.push((
                    name.local.to_string(),
                    attrs
                        .borrow()
                        .iter()
                        .map(|attr| attr.name.local.to_string())
                        .collect(),
                ));
            }
        }
        result
    }

    #[test]
    fn retains_trusted_scripts_handlers_resources_and_forms() {
        let input = "<script src='https://cdn.test/chart.js'></script><script>cockpit.select({answer: 4})</script><link rel=stylesheet href='https://cdn.test/theme.css'><style>@import 'https://cdn.test/import.css'; p {background:url(https://cdn.test/bg);color:red}</style><form action='https://example.test'><input name=answer><button onclick='cockpit.select(this.form.answer.value)' formaction='https://example.test/submit'>Choose</button></form><img src='https://cdn.test/image.png' srcset='data:image/png;base64,AA== 1x, https://cdn.test/image-2.png 2x'><p style=\"background:url('https://cdn.test/bg')\"><a href='https://example.test' target=_blank>Link</a></p>";
        let output = sanitize(input).unwrap();
        for retained in [
            "src=\"https://cdn.test/chart.js\"",
            "cockpit.select({answer: 4})",
            "href=\"https://cdn.test/theme.css\"",
            "@import 'https://cdn.test/import.css'",
            "background:url(https://cdn.test/bg)",
            "action=\"https://example.test\"",
            "onclick=\"cockpit.select(this.form.answer.value)\"",
            "formaction=\"https://example.test/submit\"",
            "src=\"https://cdn.test/image.png\"",
            "srcset=\"data:image/png;base64,AA== 1x, https://cdn.test/image-2.png 2x\"",
            "background:url('https://cdn.test/bg')",
            "href=\"https://example.test\"",
            "target=\"_blank\"",
        ] {
            assert!(
                output.document.contains(retained),
                "{retained}: {}",
                output.document
            );
        }
        assert!(output.warnings.is_empty());
    }

    #[test]
    fn removes_only_author_document_policy_and_base() {
        let output = sanitize("<base href='https://example.test/'><meta http-equiv=refresh content='0;url=https://example.test'><meta HTTP-EQUIV=Content-Security-Policy content=\"script-src 'none'\"><meta name=description content=kept><template><base href='https://example.test/template'><meta http-equiv=refresh content=0></template><svg><animate attributeName=x/><foreignObject><script>author()</script></foreignObject></svg><iframe src='https://example.test/frame'></iframe><object data='https://example.test/object'></object>").unwrap();
        assert!(!output.document.contains("<base"));
        assert!(!output.document.contains("http-equiv"));
        for retained in [
            "<meta name=\"description\"",
            "<template>",
            "<animate",
            "<foreignObject>",
            "<script>author()",
            "<iframe",
            "<object",
        ] {
            assert!(
                output.document.contains(retained),
                "{retained}: {}",
                output.document
            );
        }
        assert_eq!(output.warnings.len(), 2);
        assert!(output.warnings.iter().all(|warning| warning.starts_with("host_policy_preserved:") && warning.len() < 128));
    }

    #[test]
    fn bounds_policy_warnings_without_echoing_author_text() {
        let input = format!(
            "<base href='https://example.test/{}'>{}",
            "\u{202e}".repeat(1000),
            "<meta http-equiv=refresh content='0;url=https://example.test'>".repeat(30)
        );
        let output = sanitize(&input).unwrap();
        assert_eq!(output.warnings.len(), 2);
        assert!(
            output
                .warnings
                .iter()
                .all(|warning| warning.is_ascii() && warning.len() < 128)
        );
    }

    #[test]
    fn rebuilds_malformed_nesting_and_sanitizes_title_default() {
        let output = sanitize("<title>  Useful &amp; safe\u{202e}\n title </title><table><p onclick=author()>text<tr><td>cell</table>").unwrap();
        assert_eq!(output.title.as_deref(), Some("Useful & safe title"));
        assert!(output.document.contains("cell"));
        assert!(
            elements(&output.document)
                .iter()
                .any(|(_, attrs)| attrs.contains(&"onclick".to_owned())),
            "rendered document: {}",
            output.document,
        );
        assert_eq!(sanitize("<p>no title</p>").unwrap().title, None);
        assert_eq!(
            sanitize(&format!("<title>{}</title>", "界".repeat(81)))
                .unwrap()
                .title,
            Some(format!("{}…", "界".repeat(79)))
        );
    }

    #[test]
    fn bounds_serialized_output_when_html_escaping_expands_input() {
        let expansion = format!("<p>{}</p>", "&".repeat(WIDGET_MAX_HTML_BYTES / 2));
        assert_eq!(sanitize(&expansion).unwrap_err().code, "widget_too_large");
        let overhead = sanitize("<p></p>").unwrap().document.len();
        let exact = format!("<p>{}</p>", "a".repeat(WIDGET_MAX_HTML_BYTES - overhead));
        assert_eq!(
            sanitize(&exact).unwrap().document.len(),
            WIDGET_MAX_HTML_BYTES
        );
        let over = format!(
            "<p>{}</p>",
            "a".repeat(WIDGET_MAX_HTML_BYTES - overhead + 1)
        );
        assert_eq!(sanitize(&over).unwrap_err().code, "widget_too_large");
    }

    #[test]
    fn enforces_size_node_and_depth_limits_including_template_contents() {
        assert_eq!(
            sanitize(&" ".repeat(WIDGET_MAX_HTML_BYTES + 1))
                .err()
                .unwrap()
                .code,
            "widget_too_large"
        );
        let too_many = format!("<template>{}</template>", "<!---->".repeat(MAX_NODES));
        assert_eq!(
            sanitize(&too_many).err().unwrap().code,
            "widget_too_complex"
        );
        let too_deep = format!(
            "{}text{}",
            "<div>".repeat(MAX_DEPTH),
            "</div>".repeat(MAX_DEPTH)
        );
        assert_eq!(
            sanitize(&too_deep).err().unwrap().code,
            "widget_too_complex"
        );
        let very_deep = format!("{}text{}", "<div>".repeat(10_000), "</div>".repeat(10_000));
        assert_eq!(sanitize(&very_deep).unwrap_err().code, "widget_too_complex");
        let bounded = format!("{}text{}", "<div>".repeat(250), "</div>".repeat(250));
        assert!(sanitize(&bounded).unwrap().document.contains("text"));
    }
}
