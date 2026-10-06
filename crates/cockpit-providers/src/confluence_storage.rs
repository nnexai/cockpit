//! Local, read-only rendering of Confluence's storage format. Never emit raw HTML.
use std::borrow::Cow;

use cockpit_core::InspectionError;
use cockpit_core::sources::SourceAttachment;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use url::Url;

const MAX_STORAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_MARKDOWN_BYTES: usize = 1024 * 1024;
// Room for block separators before normalization; the final Markdown cap is exact.
const MAX_RENDER_BYTES: usize = 2 * MAX_MARKDOWN_BYTES;
const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 200_000;

pub(crate) struct StorageContext<'a> {
    pub instance: &'a str,
    pub space_key: &'a str,
    pub attachments: &'a [SourceAttachment],
}

struct Element<'a> {
    name: &'static str,
    attrs: Vec<(&'static str, String)>,
    children: Vec<Node<'a>>,
}

enum Node<'a> {
    Text(Cow<'a, str>),
    Element(Element<'a>),
}

impl Element<'_> {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    fn child(&self, name: &str) -> Option<&Element<'_>> {
        self.children.iter().find_map(|node| match node {
            Node::Element(element) if element.name == name => Some(element),
            _ => None,
        })
    }
}

fn contract() -> InspectionError {
    InspectionError {
        code: "source_provider_contract".into(),
        message: "Confluence returned malformed storage content".into(),
    }
}

fn truncated() -> InspectionError {
    InspectionError {
        code: "source_truncated".into(),
        message: "Confluence storage content exceeds the local rendering limit".into(),
    }
}

/// Preserve unknown named entities literally, but reject malformed numeric references.
fn entities(value: &str) -> Result<Cow<'_, str>, InspectionError> {
    if !value.contains('&') {
        return Ok(Cow::Borrowed(value));
    }
    let mut result = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find('&') {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start..];
        let end = remaining.find(';').ok_or_else(contract)?;
        let reference = &remaining[..=end];
        let name = &remaining[1..end];
        if (!name.starts_with('#') && !xml_name(name))
            || name.contains(['&', '<', ' ', '\n', '\r', '\t'])
        {
            return Err(contract());
        }
        if name.starts_with('#') || quick_xml::escape::resolve_predefined_entity(name).is_some() {
            let decoded = quick_xml::escape::unescape(reference).map_err(|_| contract())?;
            if !decoded.chars().all(xml_char) {
                return Err(contract());
            }
            result.push_str(&decoded);
        } else {
            result.push_str(reference);
        }
        remaining = &remaining[end + 1..];
    }
    result.push_str(remaining);
    Ok(Cow::Owned(result))
}

fn xml_char(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}

fn xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_alphabetic() || matches!(ch, ':' | '_'))
        && chars.all(|ch| ch.is_alphanumeric() || matches!(ch, ':' | '_' | '-' | '.'))
}

fn element<'a>(start: BytesStart<'a>) -> Result<Element<'a>, InspectionError> {
    // Retain only renderer-relevant metadata and intern names as constants.
    // Unknown elements simply preserve their readable children.
    const TAGS: &[&str] = &[
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "p",
        "div",
        "br",
        "hr",
        "strong",
        "b",
        "em",
        "i",
        "s",
        "del",
        "code",
        "pre",
        "ul",
        "ol",
        "li",
        "blockquote",
        "table",
        "thead",
        "tbody",
        "tfoot",
        "tr",
        "th",
        "td",
        "a",
        "img",
        "time",
        "script",
        "style",
        "iframe",
        "object",
        "col",
        "colgroup",
        "ac:placeholder",
        "ac:structured-macro",
        "ac:macro",
        "ac:parameter",
        "ac:rich-text-body",
        "ac:plain-text-body",
        "ac:link",
        "ac:image",
        "ac:link-body",
        "ac:plain-text-link-body",
        "ac:task-list",
        "ac:task",
        "ac:task-status",
        "ac:task-body",
        "ac:emoticon",
        "ri:user",
        "ri:page",
        "ri:attachment",
        "ri:url",
    ];
    const ATTRS: &[&str] = &[
        "href",
        "src",
        "alt",
        "start",
        "datetime",
        "ac:name",
        "ac:anchor",
        "ac:alt",
        "ac:emoji-fallback",
        "ri:content-title",
        "ri:space-key",
        "ri:filename",
        "ri:value",
    ];
    let raw_name = start.name();
    let raw_name = std::str::from_utf8(raw_name.as_ref()).map_err(|_| contract())?;
    if !xml_name(raw_name) {
        return Err(contract());
    }
    let name = TAGS
        .iter()
        .copied()
        .find(|name| *name == raw_name)
        .unwrap_or("unknown");
    let mut attrs = Vec::new();
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|_| contract())?;
        let key = std::str::from_utf8(attribute.key.as_ref()).map_err(|_| contract())?;
        if !xml_name(key) || attribute.value.contains(&b'<') {
            return Err(contract());
        }
        let value =
            entities(std::str::from_utf8(attribute.value.as_ref()).map_err(|_| contract())?)?;
        if let Some(key) = ATTRS.iter().copied().find(|name| *name == key) {
            attrs.push((key, value.into_owned()));
        }
    }
    Ok(Element {
        name,
        attrs,
        children: Vec::new(),
    })
}

fn parse(storage: &str) -> Result<Element<'_>, InspectionError> {
    let mut reader = Reader::from_str(storage);
    reader.config_mut().check_comments = true;
    let mut stack = vec![Element {
        name: "root",
        attrs: Vec::new(),
        children: Vec::new(),
    }];
    let mut depth = 0usize;
    let mut hidden_depth = None;
    let mut nodes = 0usize;
    let mut finished = false;
    loop {
        let event = reader.read_event().map_err(|_| contract())?;
        if finished && !matches!(event, Event::Eof) {
            return Err(contract());
        }
        match event {
            Event::Start(start) => {
                depth += 1;
                let element = element(start)?;
                if hidden_depth.is_none()
                    && matches!(
                        element.name,
                        "script" | "style" | "iframe" | "object" | "ac:placeholder"
                    )
                {
                    hidden_depth = Some(depth);
                }
                if depth <= MAX_DEPTH {
                    stack.push(element);
                }
            }
            Event::Empty(start) => {
                let element = element(start)?;
                if hidden_depth.is_none()
                    && depth < MAX_DEPTH
                    && !matches!(
                        element.name,
                        "script" | "style" | "iframe" | "object" | "ac:placeholder"
                    )
                {
                    nodes += 1;
                    stack
                        .last_mut()
                        .ok_or_else(contract)?
                        .children
                        .push(Node::Element(element));
                }
            }
            Event::End(_) => {
                if depth == 0 {
                    return Err(contract());
                }
                if depth <= MAX_DEPTH {
                    let element = stack.pop().ok_or_else(contract)?;
                    if hidden_depth.is_none() {
                        nodes += 1;
                        stack
                            .last_mut()
                            .ok_or_else(contract)?
                            .children
                            .push(Node::Element(element));
                    }
                }
                if hidden_depth == Some(depth) {
                    hidden_depth = None;
                }
                depth -= 1;
                finished = depth == 0;
            }
            Event::Text(text) => {
                if text.windows(3).any(|bytes| bytes == b"]]>") {
                    return Err(contract());
                }
                if hidden_depth.is_none() {
                    nodes += 1;
                    let value = match text.into_inner() {
                        Cow::Borrowed(bytes) => {
                            Cow::Borrowed(std::str::from_utf8(bytes).map_err(|_| contract())?)
                        }
                        Cow::Owned(bytes) => {
                            Cow::Owned(String::from_utf8(bytes).map_err(|_| contract())?)
                        }
                    };
                    stack
                        .last_mut()
                        .ok_or_else(contract)?
                        .children
                        .push(Node::Text(value));
                }
            }
            Event::CData(text) => {
                if hidden_depth.is_none() {
                    nodes += 1;
                    let value = match text.into_inner() {
                        Cow::Borrowed(bytes) => {
                            Cow::Borrowed(std::str::from_utf8(bytes).map_err(|_| contract())?)
                        }
                        Cow::Owned(bytes) => {
                            Cow::Owned(String::from_utf8(bytes).map_err(|_| contract())?)
                        }
                    };
                    stack
                        .last_mut()
                        .ok_or_else(contract)?
                        .children
                        .push(Node::Text(value));
                }
            }
            Event::GeneralRef(reference) => {
                let name = reference.decode().map_err(|_| contract())?;
                let character = reference.resolve_char_ref().map_err(|_| contract())?;
                if character.is_some_and(|ch| !xml_char(ch))
                    || (character.is_none() && !xml_name(&name))
                {
                    return Err(contract());
                }
                if hidden_depth.is_none() {
                    nodes += 1;
                    let value = if let Some(ch) = character {
                        ch.to_string()
                    } else if let Some(value) = quick_xml::escape::resolve_predefined_entity(&name)
                    {
                        value.to_owned()
                    } else {
                        format!("&{name};")
                    };
                    stack
                        .last_mut()
                        .ok_or_else(contract)?
                        .children
                        .push(Node::Text(Cow::Owned(value)));
                }
            }
            Event::Eof => break,
            Event::Comment(_) => {}
            // Storage is an XML fragment, never a document with entities or processing instructions.
            Event::Decl(_) | Event::DocType(_) | Event::PI(_) => return Err(contract()),
        }
        if nodes > MAX_NODES {
            return Err(truncated());
        }
    }
    if depth != 0 || stack.len() != 1 {
        return Err(contract());
    }
    stack.pop().ok_or_else(contract)
}

#[derive(Default)]
struct Output(String);

impl Output {
    fn push(&mut self, value: &str) -> Result<(), InspectionError> {
        if self.0.len().saturating_add(value.len()) > MAX_RENDER_BYTES {
            return Err(truncated());
        }
        self.0.push_str(value);
        Ok(())
    }

    fn paragraph(&mut self) -> Result<(), InspectionError> {
        if !self.0.is_empty() && !self.0.ends_with("\n\n") {
            if !self.0.ends_with('\n') {
                self.push("\n")?;
            }
            self.push("\n")?;
        }
        Ok(())
    }
}

fn raw_text(element: &Element<'_>, output: &mut Output) -> Result<(), InspectionError> {
    for child in &element.children {
        match child {
            Node::Text(text) => output.push(text)?,
            Node::Element(child) => raw_text(child, output)?,
        }
    }
    Ok(())
}

fn escaped(value: &str, output: &mut Output) -> Result<(), InspectionError> {
    let mut space = false;
    for ch in value.chars() {
        if ch.is_whitespace() {
            if !space {
                output.push(" ")?;
            }
            space = true;
            continue;
        }
        space = false;
        match ch {
            '<' => output.push("&lt;")?,
            '>' => output.push("&gt;")?,
            '\\' | '`' | '*' | '_' | '[' | ']' | '!' | '#' => {
                output.push("\\")?;
                output.push(ch.encode_utf8(&mut [0; 4]))?;
            }
            _ => output.push(ch.encode_utf8(&mut [0; 4]))?,
        }
    }
    Ok(())
}

fn safe_url(value: &str, context: &StorageContext<'_>, image: bool) -> Option<String> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().any(|ch| ch.is_control() || ch == '\\')
    {
        return None;
    }
    let url = match Url::parse(value) {
        Ok(url) => url,
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            Url::parse(&format!("{}/", context.instance))
                .ok()?
                .join(value)
                .ok()?
        }
        Err(_) => return None,
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.scheme(), "http" | "https" | "mailto")
        || (image && url.scheme() == "mailto")
    {
        return None;
    }
    // Parentheses and angle brackets must not escape the Markdown destination.
    if url.as_str().contains(['(', ')', '<', '>']) {
        Some(markdown_destination(url.as_str()))
    } else {
        Some(url.into())
    }
}

fn markdown_destination(value: &str) -> String {
    let extra = value
        .bytes()
        .filter(|byte| matches!(*byte, b'(' | b')' | b'<' | b'>'))
        .count()
        * 2;
    let mut destination = String::with_capacity(value.len() + extra);
    for ch in value.chars() {
        match ch {
            '(' => destination.push_str("%28"),
            ')' => destination.push_str("%29"),
            '<' => destination.push_str("%3C"),
            '>' => destination.push_str("%3E"),
            _ => destination.push(ch),
        }
    }
    destination
}

fn attachment_url(filename: &str, context: &StorageContext<'_>, image: bool) -> Option<String> {
    context
        .attachments
        .iter()
        .find(|attachment| attachment.title == filename)
        .and_then(|attachment| attachment.source_url.as_deref())
        .and_then(|url| safe_url(url, context, image))
}

fn link(
    label: &str,
    destination: Option<&str>,
    image: bool,
    output: &mut Output,
) -> Result<(), InspectionError> {
    if let Some(destination) = destination {
        output.push(if image { "![" } else { "[" })?;
        output.push(label)?;
        output.push("](")?;
        output.push(destination)?;
        output.push(")")
    } else {
        output.push(label)
    }
}

fn fence(body: &str, language: &str, output: &mut Output) -> Result<(), InspectionError> {
    let longest = body.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
    let delimiter = "`".repeat(3.max(longest + 1));
    output.paragraph()?;
    output.push(&delimiter)?;
    if language
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+' | '.'))
    {
        output.push(language)?;
    }
    output.push("\n")?;
    output.push(body)?;
    if !body.ends_with('\n') {
        output.push("\n")?;
    }
    output.push(&delimiter)?;
    output.push("\n\n")
}

fn inline_code(body: &str, output: &mut Output) -> Result<(), InspectionError> {
    let longest = body.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
    let delimiter = "`".repeat(1.max(longest + 1));
    let pad = body.starts_with(['`', ' ']) || body.ends_with(['`', ' ']);
    output.push(&delimiter)?;
    if pad {
        output.push(" ")?;
    }
    for (index, line) in body.lines().enumerate() {
        if index != 0 {
            output.push(" ")?;
        }
        output.push(line)?;
    }
    if pad {
        output.push(" ")?;
    }
    output.push(&delimiter)
}

fn children(
    element: &Element<'_>,
    context: &StorageContext<'_>,
    depth: usize,
    output: &mut Output,
) -> Result<(), InspectionError> {
    for child in &element.children {
        match child {
            Node::Text(text) => {
                if !text.trim().is_empty() || (!output.0.is_empty() && !output.0.ends_with('\n')) {
                    escaped(text, output)?;
                }
            }
            Node::Element(element) => render(element, context, depth, output)?,
        }
    }
    Ok(())
}

fn rendered(
    element: &Element<'_>,
    context: &StorageContext<'_>,
    depth: usize,
) -> Result<String, InspectionError> {
    let mut output = Output::default();
    children(element, context, depth, &mut output)?;
    Ok(output.0)
}

fn quote(body: &str, output: &mut Output) -> Result<(), InspectionError> {
    output.paragraph()?;
    for line in body.trim().lines() {
        output.push("> ")?;
        output.push(line)?;
        output.push("\n")?;
    }
    output.push("\n")
}

fn parameter(element: &Element<'_>, name: &str) -> Result<Option<String>, InspectionError> {
    for child in &element.children {
        if let Node::Element(child) = child {
            if child.name == "ac:parameter" && child.attr("ac:name") == Some(name) {
                let mut output = Output::default();
                raw_text(child, &mut output)?;
                return Ok(Some(output.0));
            }
        }
    }
    Ok(None)
}

fn macro_body(
    element: &Element<'_>,
    context: &StorageContext<'_>,
    depth: usize,
    output: &mut Output,
) -> Result<(), InspectionError> {
    let name = element.attr("ac:name").unwrap_or("");
    let rich = element.child("ac:rich-text-body");
    match name {
        "code" | "noformat" => {
            let mut body = Output::default();
            if let Some(plain) = element.child("ac:plain-text-body") {
                raw_text(plain, &mut body)?;
            }
            let language = parameter(element, "language")?.unwrap_or_default();
            fence(&body.0, &language, output)
        }
        "info" | "note" | "warning" | "tip" | "panel" => {
            let default = match name {
                "info" => "Info",
                "note" => "Note",
                "warning" => "Warning",
                "tip" => "Tip",
                _ => "Panel",
            };
            let title = parameter(element, "title")?
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| default.into());
            let mut body = Output::default();
            body.push("**")?;
            escaped(title.trim(), &mut body)?;
            body.push(":**")?;
            if let Some(rich) = rich {
                body.push("\n\n")?;
                children(rich, context, depth, &mut body)?;
            }
            quote(&body.0, output)
        }
        "expand" => {
            let title = parameter(element, "title")?.unwrap_or_else(|| "Expand".into());
            output.paragraph()?;
            output.push("**")?;
            escaped(title.trim(), output)?;
            output.push("**\n\n")?;
            if let Some(rich) = rich {
                children(rich, context, depth, output)?;
            }
            output.paragraph()
        }
        _ => {
            if let Some(rich) = rich {
                children(rich, context, depth, output)?;
            } else {
                // Preserve explicit source links without inventing dynamic macro results.
                for child in &element.children {
                    if let Node::Element(child) = child {
                        if matches!(child.name, "a" | "ac:link") {
                            render(child, context, depth, output)?;
                        }
                    }
                }
                if let Some(url) = parameter(element, "url")? {
                    if let Some(destination) = safe_url(&url, context, false) {
                        let mut label = Output::default();
                        escaped(&url, &mut label)?;
                        link(&label.0, Some(&destination), false, output)?;
                    }
                }
            }
            Ok(())
        }
    }
}

fn resource_link(
    element: &Element<'_>,
    context: &StorageContext<'_>,
    depth: usize,
    image: bool,
    output: &mut Output,
) -> Result<(), InspectionError> {
    let body = element
        .child("ac:link-body")
        .or_else(|| element.child("ac:plain-text-link-body"));
    let mut label = if let Some(body) = body {
        rendered(body, context, depth)?
    } else {
        String::new()
    };
    let mut destination = None;
    let mut fallback = String::new();
    if let Some(page) = element.child("ri:page") {
        fallback = page.attr("ri:content-title").unwrap_or("Page").into();
        let space = page.attr("ri:space-key").unwrap_or(context.space_key);
        if let Ok(mut url) = Url::parse(context.instance) {
            if let Ok(mut path) = url.path_segments_mut() {
                path.pop_if_empty()
                    .push("display")
                    .push(space)
                    .push(&fallback);
            }
            if let Some(anchor) = element.attr("ac:anchor") {
                url.set_fragment(Some(anchor));
            }
            destination = safe_url(url.as_str(), context, image);
        }
    } else if let Some(attachment) = element.child("ri:attachment") {
        fallback = attachment
            .attr("ri:filename")
            .unwrap_or("Attachment")
            .into();
        // A reference to another page's attachment cannot use this page's title lookup.
        if attachment.child("ri:page").is_none() {
            destination = attachment_url(&fallback, context, image);
        }
        if image && destination.is_none() && safe_url(&fallback, context, true).is_some() {
            if let Ok(mut url) = Url::parse(context.instance) {
                if let Ok(mut path) = url.path_segments_mut() {
                    path.clear().push(&fallback);
                }
                destination = Some(markdown_destination(&url.path()[1..]));
            }
        }
    } else if let Some(url) = element.child("ri:url") {
        let value = url.attr("ri:value").unwrap_or("");
        fallback = if image { String::new() } else { value.into() };
        destination = safe_url(value, context, image);
    } else if element.child("ri:user").is_some() {
        fallback = "@user".into();
    } else if let Some(anchor) = element.attr("ac:anchor") {
        fallback = anchor.into();
        destination = safe_url(&format!("#{anchor}"), context, image);
    }
    if image {
        if let Some(alt) = element.attr("ac:alt") {
            fallback = alt.into();
        }
    }
    if label.trim().is_empty() {
        let mut escaped_label = Output::default();
        escaped(&fallback, &mut escaped_label)?;
        label = escaped_label.0;
    }
    link(label.trim(), destination.as_deref(), image, output)
}

fn list(
    element: &Element<'_>,
    context: &StorageContext<'_>,
    depth: usize,
    output: &mut Output,
) -> Result<(), InspectionError> {
    output.paragraph()?;
    let mut index = element
        .attr("start")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    for child in &element.children {
        if let Node::Element(item) = child {
            if item.name != "li" {
                continue;
            }
            let indent = "  ".repeat(depth);
            output.push(&indent)?;
            if element.name == "ol" {
                output.push(&format!("{index}. "))?;
            } else {
                output.push("- ")?;
            }
            let mut body = Output::default();
            for node in &item.children {
                match node {
                    Node::Element(nested) if matches!(nested.name, "ul" | "ol") => {
                        if !body.0.ends_with('\n') {
                            body.push("\n")?;
                        }
                        // Render nested lists into their own output, then keep their structural indent.
                        let mut nested_output = Output::default();
                        list(nested, context, depth + 1, &mut nested_output)?;
                        body.push(nested_output.0.trim_end())?;
                        body.push("\n")?;
                    }
                    Node::Element(child) => render(child, context, depth + 1, &mut body)?,
                    Node::Text(text) => escaped(text, &mut body)?,
                }
            }
            let body = body.0.trim();
            for (line_index, line) in body.lines().enumerate() {
                if line_index != 0 {
                    output.push("\n")?;
                    if !line.starts_with("  ") {
                        output.push(&indent)?;
                        output.push("  ")?;
                    }
                }
                output.push(line.trim_end())?;
            }
            output.push("\n")?;
            index = index.saturating_add(1);
        }
    }
    output.push("\n")
}

fn table_rows<'a, 's>(element: &'a Element<'s>, rows: &mut Vec<&'a Element<'s>>) {
    for child in &element.children {
        if let Node::Element(child) = child {
            match child.name {
                "tr" => rows.push(child),
                "thead" | "tbody" | "tfoot" => table_rows(child, rows),
                _ => {}
            }
        }
    }
}

fn table(
    element: &Element<'_>,
    context: &StorageContext<'_>,
    depth: usize,
    output: &mut Output,
) -> Result<(), InspectionError> {
    let mut rows = Vec::new();
    table_rows(element, &mut rows);
    let columns = rows
        .iter()
        .map(|row| {
            row.children
                .iter()
                .filter(
                    |node| matches!(node, Node::Element(cell) if matches!(cell.name, "td" | "th")),
                )
                .count()
        })
        .max()
        .unwrap_or(0);
    if columns == 0 {
        return Ok(());
    }
    output.paragraph()?;
    for (index, row) in rows.iter().enumerate() {
        output.push("|")?;
        let mut count = 0;
        for cell in &row.children {
            if let Node::Element(cell) = cell {
                if !matches!(cell.name, "td" | "th") {
                    continue;
                }
                let cell = rendered(cell, context, depth)?;
                output.push(" ")?;
                for (part_index, part) in cell.split_whitespace().enumerate() {
                    if part_index != 0 {
                        output.push(" ")?;
                    }
                    output.push(&part.replace('|', "\\|"))?;
                }
                output.push(" |")?;
                count += 1;
            }
        }
        for _ in count..columns {
            output.push("  |")?;
        }
        output.push("\n")?;
        if index == 0 {
            output.push("|")?;
            for _ in 0..columns {
                output.push(" --- |")?;
            }
            output.push("\n")?;
        }
    }
    output.push("\n")
}

fn render(
    element: &Element<'_>,
    context: &StorageContext<'_>,
    depth: usize,
    output: &mut Output,
) -> Result<(), InspectionError> {
    match element.name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            output.paragraph()?;
            output.push(&"######"[..usize::from(element.name.as_bytes()[1] - b'0')])?;
            output.push(" ")?;
            children(element, context, depth, output)?;
            output.push("\n\n")
        }
        "p" | "div" => {
            output.paragraph()?;
            children(element, context, depth, output)?;
            output.push("\n\n")
        }
        "br" => output.push("\n"),
        "hr" => {
            output.paragraph()?;
            output.push("---\n\n")
        }
        "strong" | "b" | "em" | "i" | "s" | "del" => {
            let delimiter = match element.name {
                "strong" | "b" => "**",
                "em" | "i" => "_",
                _ => "~~",
            };
            output.push(delimiter)?;
            children(element, context, depth, output)?;
            output.push(delimiter)
        }
        "code" | "pre" => {
            let mut body = Output::default();
            raw_text(element, &mut body)?;
            if element.name == "pre" {
                fence(&body.0, "", output)
            } else {
                inline_code(&body.0, output)
            }
        }
        "ul" | "ol" => list(element, context, depth, output),
        "blockquote" => quote(&rendered(element, context, depth)?, output),
        "table" => table(element, context, depth, output),
        "a" => {
            let label = rendered(element, context, depth)?;
            let destination = element
                .attr("href")
                .and_then(|href| safe_url(href, context, false));
            link(&label, destination.as_deref(), false, output)
        }
        "img" => {
            let mut label = Output::default();
            escaped(element.attr("alt").unwrap_or(""), &mut label)?;
            let destination = element
                .attr("src")
                .and_then(|src| safe_url(src, context, true));
            link(&label.0, destination.as_deref(), true, output)
        }
        "ac:structured-macro" | "ac:macro" => macro_body(element, context, depth, output),
        "ac:link" => resource_link(element, context, depth, false, output),
        "ac:image" => resource_link(element, context, depth, true, output),
        "ac:task-list" => {
            output.paragraph()?;
            children(element, context, depth, output)?;
            output.paragraph()
        }
        "ac:task" => {
            let mut status = Output::default();
            if let Some(child) = element.child("ac:task-status") {
                raw_text(child, &mut status)?;
            }
            output.push(if status.0.trim() == "complete" {
                "- [x] "
            } else {
                "- [ ] "
            })?;
            if let Some(body) = element.child("ac:task-body") {
                output.push(rendered(body, context, depth)?.trim())?;
            }
            output.push("\n")
        }
        "ac:emoticon" => {
            if let Some(fallback) = element.attr("ac:emoji-fallback") {
                escaped(fallback, output)
            } else {
                output.push(":")?;
                escaped(element.attr("ac:name").unwrap_or("smile"), output)?;
                output.push(":")
            }
        }
        "time" => escaped(element.attr("datetime").unwrap_or(""), output),
        "ac:placeholder" | "ac:parameter" | "ri:user" | "ri:page" | "ri:attachment" | "ri:url"
        | "col" | "colgroup" => Ok(()),
        _ => children(element, context, depth, output),
    }
}

/// Bound XML parsing and Markdown rendering independently. Deep markup is flattened,
/// never recursively traversed beyond MAX_DEPTH. No network calls or active HTML.
pub(crate) fn storage_to_markdown(
    storage: &str,
    context: &StorageContext<'_>,
) -> Result<String, InspectionError> {
    if storage.len() > MAX_STORAGE_BYTES {
        return Err(truncated());
    }
    if !storage.chars().all(xml_char) {
        return Err(contract());
    }
    let wrapped = format!("<root>{storage}</root>");
    let root = parse(&wrapped)?;
    let mut output = Output::default();
    children(&root, context, 0, &mut output)?;
    // Keep code's whitespace intact while removing paragraph whitespace/churn.
    let mut normalized = Output::default();
    let mut blank_lines = 0;
    let mut code_fence = 0;
    for line in output.0.trim().lines() {
        let mut fence_line = line.trim_start();
        while let Some(rest) = fence_line.strip_prefix('>') {
            fence_line = rest.trim_start();
        }
        let backticks = fence_line.bytes().take_while(|byte| *byte == b'`').count();
        let in_code = code_fence != 0;
        if backticks >= 3 {
            if code_fence == 0
                && fence_line[backticks..]
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+' | '.'))
            {
                code_fence = backticks;
            } else if backticks >= code_fence && fence_line[backticks..].trim().is_empty() {
                code_fence = 0;
            }
        }
        let line = if in_code { line } else { line.trim_end() };
        if !in_code && line.is_empty() {
            blank_lines += 1;
            if blank_lines > 1 {
                continue;
            }
        } else {
            blank_lines = 0;
        }
        if !normalized.0.is_empty() {
            normalized.push("\n")?;
        }
        normalized.push(line)?;
    }
    normalized.0.truncate(normalized.0.trim_end().len());
    if normalized.0.len() > MAX_MARKDOWN_BYTES {
        return Err(truncated());
    }
    Ok(normalized.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> StorageContext<'static> {
        StorageContext {
            instance: "https://example.atlassian.net/wiki",
            space_key: "ENG",
            attachments: &[],
        }
    }

    fn convert(storage: &str) -> String {
        storage_to_markdown(storage, &context()).expect("valid storage")
    }

    fn attachment(title: &str, url: &str) -> SourceAttachment {
        SourceAttachment {
            id: "att42".into(),
            title: title.into(),
            media_type: None,
            size: None,
            source_url: Some(url.into()),
            source_revision: None,
            path: None,
            not_downloaded: None,
        }
    }

    #[test]
    fn headings_formatting_paragraphs_and_entities() {
        assert_eq!(
            convert(
                "<h1>One</h1><h2>Two</h2><h3>Three</h3><h4>Four</h4><h5>Five</h5><h6>Six</h6><p><b>bold</b> <em>emphasis</em> <del>gone</del> <u>plain</u><span> text</span><br/>next</p><hr/><p>A&nbsp;B &rsquo; &unknown; &amp; &#x1f600;</p>"
            ),
            "# One\n\n## Two\n\n### Three\n\n#### Four\n\n##### Five\n\n###### Six\n\n**bold** _emphasis_ ~~gone~~ plain text\nnext\n\n---\n\nA B ’ &unknown; & 😀"
        );
        assert_eq!(
            convert("<p><strong>strong</strong><i>italic</i><s>strike</s></p>"),
            "**strong**_italic_~~strike~~"
        );
    }

    #[test]
    fn nested_lists_and_multiline_items() {
        assert_eq!(
            convert(
                "<ul><li>First<ul><li>Child</li></ul></li><li><p>Second</p><p>Continued</p></li></ul><ol start=\"3\"><li>Third<ol><li>Nested</li></ol></li><li>Fourth</li></ol>"
            ),
            "- First\n  - Child\n- Second\n\n  Continued\n\n3. Third\n  1. Nested\n4. Fourth"
        );
    }

    #[test]
    fn tables_flatten_cells_pad_rows_and_escape_pipes() {
        assert_eq!(
            convert(
                "<table><colgroup><col/></colgroup><thead><tr><th>Name</th><th>Details</th></tr></thead><tbody><tr><td>A|B</td><td><p>First</p><p><strong>second</strong><br/>third</p></td></tr><tr><td>Only one</td></tr></tbody></table>"
            ),
            "| Name | Details |\n| --- | --- |\n| A\\|B | First **second** third |\n| Only one |  |"
        );
    }

    #[test]
    fn inline_and_fenced_code_keep_literal_content() {
        assert_eq!(
            convert("<p><code>a ` b</code></p><pre>line 1\n\n\n  line 2  </pre>"),
            "``a ` b``\n\n```\nline 1\n\n\n  line 2  \n```"
        );
        assert_eq!(
            convert(
                "<ac:structured-macro ac:name=\"code\"><ac:parameter ac:name=\"language\">rust</ac:parameter><ac:plain-text-body><![CDATA[let s = \"<tag> & ```\";\na]]]]><![CDATA[>b]]></ac:plain-text-body></ac:structured-macro>"
            ),
            "````rust\nlet s = \"<tag> & ```\";\na]]>b\n````"
        );
        assert_eq!(
            convert(
                "<ac:structured-macro ac:name=\"noformat\"><ac:plain-text-body><![CDATA[a & b]]></ac:plain-text-body></ac:structured-macro>"
            ),
            "```\na & b\n```"
        );
        assert_eq!(
            convert(
                "<ac:structured-macro ac:name=\"code\"><ac:parameter ac:name=\"language\">bad`language</ac:parameter><ac:plain-text-body>x</ac:plain-text-body></ac:structured-macro>"
            ),
            "```\nx\n```"
        );
    }

    #[test]
    fn panels_expand_and_unknown_macros_preserve_rich_content() {
        for (name, title) in [
            ("info", "Info"),
            ("note", "Note"),
            ("warning", "Warning"),
            ("tip", "Tip"),
            ("panel", "Panel"),
        ] {
            let storage = format!(
                "<ac:structured-macro ac:name=\"{name}\"><ac:rich-text-body><p>Visible <strong>body</strong></p></ac:rich-text-body></ac:structured-macro>"
            );
            assert_eq!(
                convert(&storage),
                format!("> **{title}:**\n>\n> Visible **body**")
            );
        }
        assert_eq!(
            convert(
                "<ac:structured-macro ac:name=\"panel\"><ac:parameter ac:name=\"title\">Custom</ac:parameter><ac:rich-text-body><p>Body</p></ac:rich-text-body></ac:structured-macro><ac:structured-macro ac:name=\"expand\"><ac:parameter ac:name=\"title\">More</ac:parameter><ac:rich-text-body><p>Expanded</p></ac:rich-text-body></ac:structured-macro>"
            ),
            "> **Custom:**\n>\n> Body\n\n**More**\n\nExpanded"
        );
        assert_eq!(
            convert(
                "<ac:structured-macro ac:name=\"unknown\"><ac:rich-text-body><p>Keep <a href=\"https://docs.example/a\">source</a></p></ac:rich-text-body></ac:structured-macro><ac:structured-macro ac:name=\"toc\"/><ac:structured-macro ac:name=\"children\"/><ac:structured-macro ac:name=\"jira\"><ac:parameter ac:name=\"url\">https://jira.example/browse/TEST-1</ac:parameter></ac:structured-macro>"
            ),
            "Keep [source](https://docs.example/a)\n\n[https://jira.example/browse/TEST-1](https://jira.example/browse/TEST-1)"
        );
    }

    #[test]
    fn links_page_refs_images_and_users() {
        let attachments = [attachment(
            "flow.png",
            "https://example.atlassian.net/wiki/download/attachments/42/flow.png",
        )];
        let context = StorageContext {
            attachments: &attachments,
            ..context()
        };
        let storage = "<p><a href=\"/wiki/docs?a=1&amp;b=2\">Docs</a> <a href=\"mailto:a@example.com\">Mail</a></p><p><ac:link><ri:page ri:content-title=\"A/B &amp; C\"/><ac:link-body>Page <strong>label</strong></ac:link-body></ac:link></p><p><ac:link><ri:page ri:space-key=\"OTHER\" ri:content-title=\"Start Here\"/></ac:link></p><p><ac:link><ri:attachment ri:filename=\"flow.png\"/></ac:link> <ac:link><ri:attachment ri:filename=\"missing.pdf\"/></ac:link></p><p><ac:image ac:alt=\"Flow\"><ri:attachment ri:filename=\"flow.png\"/></ac:image> <ac:image><ri:url ri:value=\"https://images.example/flow.png\"/></ac:image></p><p><ac:link><ri:user ri:account-id=\"private-id\"/></ac:link> <ac:link><ri:user ri:userkey=\"private-key\"/><ac:plain-text-link-body><![CDATA[Team member]]></ac:plain-text-link-body></ac:link></p>";
        assert_eq!(
            storage_to_markdown(storage, &context).expect("valid refs"),
            "[Docs](https://example.atlassian.net/wiki/docs?a=1&b=2) [Mail](mailto:a@example.com)\n\n[Page **label**](https://example.atlassian.net/wiki/display/ENG/A%2FB%20&%20C)\n\n[Start Here](https://example.atlassian.net/wiki/display/OTHER/Start%20Here)\n\n[flow.png](https://example.atlassian.net/wiki/download/attachments/42/flow.png) missing.pdf\n\n![Flow](https://example.atlassian.net/wiki/download/attachments/42/flow.png) ![](https://images.example/flow.png)\n\n@user Team member"
        );
        assert_eq!(
            convert("<img alt=\"Picture\" src=\"https://images.example/a(b).png\"/>"),
            "![Picture](https://images.example/a%28b%29.png)"
        );
        assert_eq!(
            convert("<ac:image><ri:attachment ri:filename=\"missing image.png\"/></ac:image>"),
            "![missing image.png](missing%20image.png)"
        );
        assert_eq!(
            convert(
                "<ac:link><ri:attachment ri:filename=\"flow.png\"><ri:page ri:content-title=\"Other\"/></ri:attachment></ac:link>"
            ),
            "flow.png"
        );
    }

    #[test]
    fn task_lists_dates_placeholders_comments_and_emoticons() {
        assert_eq!(
            convert(
                "<ac:task-list><ac:task><ac:task-id>99</ac:task-id><ac:task-status>incomplete</ac:task-status><ac:task-body>Open</ac:task-body></ac:task><ac:task><ac:task-status>complete</ac:task-status><ac:task-body><strong>Done</strong></ac:task-body></ac:task></ac:task-list><p><time datetime=\"2026-10-06\">ignored</time> <ac:emoticon ac:emoji-fallback=\"🙂\"/> <ac:emoticon ac:name=\"smile\"/> <ac:inline-comment-marker>Visible</ac:inline-comment-marker><ac:placeholder>Hidden</ac:placeholder></p>"
            ),
            "- [ ] Open\n- [x] **Done**\n\n2026-10-06 🙂 :smile: Visible"
        );
        assert_eq!(
            convert("<blockquote><p>Quoted</p><p>Again</p></blockquote>"),
            "> Quoted\n>\n> Again"
        );
    }

    #[test]
    fn unsafe_uris_and_active_html_are_not_emitted() {
        for uri in [
            "javascript:alert(1)",
            "data:text/html,evil",
            "vbscript:evil",
            "file:///etc/passwd",
            "https://user:pass@example.com/x",
            " javaScript:evil",
            "java&#x09;script:evil",
            "https://example.com/&#10;evil",
            "https:\\evil",
        ] {
            let storage = format!(
                "<p><a href=\"{uri}\">Readable</a><img alt=\"Image\" src=\"{uri}\"/><ac:image><ri:url ri:value=\"{uri}\"/></ac:image></p>"
            );
            assert_eq!(convert(&storage), "ReadableImage", "unsafe URI {uri}");
        }
        assert_eq!(
            convert(
                "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p><script>hidden</script><style>hidden</style><iframe>hidden</iframe><object>hidden</object><p>Safe <span onclick=\"evil()\">text</span></p>"
            ),
            "&lt;script&gt;alert(1)&lt;/script&gt;\n\nSafe text"
        );
        let attachments = [attachment("flow.png", "javascript:evil")];
        let context = StorageContext {
            attachments: &attachments,
            ..context()
        };
        assert_eq!(
            storage_to_markdown(
                "<ac:link><ri:attachment ri:filename=\"flow.png\"/></ac:link>",
                &context
            )
            .expect("safe fallback"),
            "flow.png"
        );
    }

    #[test]
    fn malformed_xml_and_external_entities_are_rejected() {
        for storage in [
            "<p>unclosed",
            "<p>x</div>",
            "<p x=\"1\" x=\"2\"/>",
            "<p>&#xZZ;</p>",
            "<p>&#0;</p>",
            "<p>&unterminated</p>",
            "<p>&bad name;</p>",
            "<p x=\"<\"/>",
            "<1bad/>",
            "</root><root>",
            "<p>]]></p>",
            "<?xml version=\"1.0\"?><p>x</p>",
            "<!DOCTYPE root [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><p>&x;</p>",
            "<p>\u{1}</p>",
        ] {
            assert_eq!(
                storage_to_markdown(storage, &context())
                    .expect_err("malformed storage")
                    .code,
                "source_provider_contract",
                "{storage}"
            );
        }
    }

    #[test]
    fn deep_markup_is_flattened_without_losing_text() {
        let storage = format!(
            "{}visible{}",
            "<span>".repeat(10_000),
            "</span>".repeat(10_000)
        );
        assert_eq!(convert(&storage), "visible");
        let storage = format!(
            "{}<script>hidden</script>visible{}",
            "<span>".repeat(100),
            "</span>".repeat(100)
        );
        assert_eq!(convert(&storage), "visible");
    }

    #[test]
    fn input_output_and_node_limits_are_bounded() {
        assert_eq!(
            storage_to_markdown(&"x".repeat(MAX_STORAGE_BYTES + 1), &context())
                .expect_err("input cap")
                .code,
            "source_truncated"
        );
        assert_eq!(
            storage_to_markdown(
                &format!("<p>{}</p>", "x".repeat(MAX_MARKDOWN_BYTES + 1)),
                &context()
            )
            .expect_err("output cap")
            .code,
            "source_truncated"
        );
        assert_eq!(
            convert(&format!("<p>{}</p>", "x".repeat(MAX_MARKDOWN_BYTES))).len(),
            MAX_MARKDOWN_BYTES
        );
        assert_eq!(
            storage_to_markdown(&"<br/>".repeat(MAX_NODES + 1), &context())
                .expect_err("node cap")
                .code,
            "source_truncated"
        );
        // Larger than the validated Cloud page; this remains below the output cap.
        let body = "visible content ".repeat(45_000);
        assert_eq!(convert(&format!("<p>{body}</p>")), body.trim_end());
    }

    #[test]
    fn dc_fixture_visible_content_matches_with_attachment_image() {
        let attachments = [attachment(
            "release-flow.png",
            "https://confluence.example.com/confluence/download/attachments/524301/release-flow.png",
        )];
        let context = StorageContext {
            instance: "https://confluence.example.com/confluence",
            space_key: "REL",
            attachments: &attachments,
        };
        let storage = "<h2>Before the release</h2><ul><li>Freeze the branch</li><li>Run the smoke suite</li></ul><ac:image ac:alt=\"Release flow\"><ri:attachment ri:filename=\"release-flow.png\"/></ac:image>";
        let expected = "## Before the release\n\n- Freeze the branch\n- Run the smoke suite\n\n![Release flow](https://confluence.example.com/confluence/download/attachments/524301/release-flow.png)";
        assert_eq!(
            storage_to_markdown(storage, &context).expect("DC content"),
            expected
        );
        assert_eq!(
            storage_to_markdown(storage, &context).expect("deterministic"),
            expected
        );
        assert_eq!(
            storage_to_markdown(
                "<ac:link><ri:page ri:content-title=\"Release Notes\"/></ac:link>",
                &context
            )
            .expect("DC page link"),
            "[Release Notes](https://confluence.example.com/confluence/display/REL/Release%20Notes)"
        );
    }
}
