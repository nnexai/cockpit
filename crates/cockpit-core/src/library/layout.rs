use crate::sources::{FrontmatterValue, SourceAsset};
use percent_encoding::percent_decode_str;
use url::Url;

pub(crate) const FILES_DIR: &str = "_files";
const SEGMENT_LIMIT: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placement {
    pub container: Vec<String>,
    pub leaf: String,
    pub document: String,
}

pub(crate) fn segment(raw: &str) -> String {
    let mut value = String::with_capacity(raw.len().min(SEGMENT_LIMIT));
    for ch in raw.chars() {
        if ch == '/' || ch == '\\' || ch == '\0' || ch.is_control() {
            value.push('-');
        } else {
            value.push(ch);
        }
    }
    if value.starts_with('.') {
        value.replace_range(..1, "_");
    }
    let value = value.trim_end_matches([' ', '.']);
    let value = if value.is_empty() { "untitled" } else { value };
    let value = trim_ending(truncate(value, SEGMENT_LIMIT));
    if value.is_empty() { "untitled".into() } else { value.to_owned() }
}
pub(crate) fn tagged(value: &str, tag: &str) -> String {
    let tag = segment(tag);
    let tag = truncate(&tag, SEGMENT_LIMIT - 14);
    let suffix = format!(" [{tag}]");
    let limit = SEGMENT_LIMIT.saturating_sub(suffix.len()).saturating_sub(3);
    let base = trim_ending(truncate(value, limit));
    format!("{}{}", if base.is_empty() { "untitled" } else { base }, suffix)
}
pub(crate) fn is_reserved(value: &str, at_root: bool) -> bool {
    matches!(value, "." | ".." | ".cockpit" | FILES_DIR)
        || (at_root && value == "README.md")
}

pub(crate) fn folder_leaf(label: &str) -> String {
    segment(label)
}

pub(crate) fn source_placement(asset: &SourceAsset) -> Placement {
    let source = &asset.source;
    let confluence = source.provider_id == "confluence"
        || (source.resource_type == "page" && field_string(asset, "space_key").is_some());
    let provider_kind = if confluence { "confluence" } else { source.provider_id.as_str() };
    let (host, base_path) = host_and_path(&source.provider_instance, confluence);
    let mut container = vec![segment(provider_kind), host];
    let id = source.canonical_id.as_str();
    let mut leaf = id.to_owned();

    match (provider_kind, source.resource_type.as_str()) {
        ("confluence", "page") => {
            let key = field_string(asset, "space_key").or_else(|| asset.container.as_ref().map(|c| c.id.clone())).unwrap_or_else(|| "space".into());
            let name = field_string(asset, "space_name").or_else(|| asset.container.as_ref().and_then(|c| c.label.split_once(" · ").map(|(_, name)| name.to_owned())));
            let space = name.map(|name| format!("{key} - {name}")).unwrap_or(key);
            container.push(segment(&space));
            container.extend(field_strings(asset, "ancestors").iter().map(|part| segment(part)));
            leaf = asset.title.clone();
        }
        ("jira", "issue") => {
            let project = id.rsplit_once('-').map(|(key, _)| key).unwrap_or(id);
            container.push(segment(project));
        }
        ("gitlab", "issue") | ("gitlab", "review") => {
            let (project, number) = split_key_number(id, if source.resource_type == "review" { '!' } else { '#' });
            append_repository(&mut container, project);
            container.push(if source.resource_type == "review" { "merge-requests".into() } else { "issues".into() });
            leaf = number.to_owned();
        }
        ("github", "issue") | ("github", "review") | ("tea", "issue") | ("tea", "review") => {
            let (project, number) = split_key_number(id, if source.resource_type == "review" { '!' } else { '#' });
            append_repository(&mut container, project);
            container.push(if source.resource_type == "review" { "pulls".into() } else { "issues".into() });
            leaf = number.to_owned();
        }
        ("tea", "wiki") => {
            let (repository, page) = id.split_once(':').unwrap_or((id, "untitled"));
            append_repository(&mut container, repository);
            container.push("wiki".into());
            let mut page_parts = page.split('/').filter(|part| !part.is_empty()).collect::<Vec<_>>();
            if let Some(last) = page_parts.pop() {
                container.extend(page_parts.into_iter().map(segment));
                leaf = last.to_owned();
            }
        }
        _ => {
            container.push(segment(&source.resource_type));
        }
    }

    // Base-path segments distinguish instances hosted below a shared authority.
    container[1] = append_base_path(&container[1], &base_path);
    let leaf = segment(&leaf);
    let document = format!("{}.md", truncate(&segment(&asset.title), SEGMENT_LIMIT - 3));
    Placement { container, leaf, document }
}

fn split_key_number(id: &str, separator: char) -> (&str, &str) {
    id.rsplit_once(separator).unwrap_or((id, id))
}

fn append_repository(container: &mut Vec<String>, repository: &str) {
    container.extend(repository.split('/').filter(|part| !part.is_empty()).map(segment));
}

fn field_string(asset: &SourceAsset, key: &str) -> Option<String> {
    asset.fields.iter().find(|field| field.key == key).and_then(|field| match &field.value {
        FrontmatterValue::String(value) => Some(value.clone()),
        _ => None,
    })
}

fn field_strings(asset: &SourceAsset, key: &str) -> Vec<String> {
    asset.fields.iter().find(|field| field.key == key).and_then(|field| match &field.value {
        FrontmatterValue::Strings(values) => Some(values.clone()),
        _ => None,
    }).unwrap_or_default()
}

fn host_and_path(instance: &str, confluence: bool) -> (String, String) {
    let Ok(url) = Url::parse(instance) else { return ("unknown".into(), String::new()) };
    let mut host = url.host_str().unwrap_or("unknown").to_owned();
    if let Some(port) = url.port() {
        host.push('_');
        host.push_str(&port.to_string());
    }
    let path = url.path().trim_matches('/');
    let path = if confluence && path == "wiki" { "" } else { path };
    let decoded = path.split('/').filter(|part| !part.is_empty()).map(|part| {
        percent_decode_str(part).decode_utf8_lossy().into_owned()
    }).collect::<Vec<_>>().join("-");
    (segment(&host), decoded)
}

fn append_base_path(host: &str, path: &str) -> String {
    if path.is_empty() { host.to_owned() } else { segment(&format!("{host}-{}", path.split('-').map(segment).collect::<Vec<_>>().join("-"))) }
}

fn truncate(value: &str, max: usize) -> &str {
    let mut end = value.len().min(max);
    while !value.is_char_boundary(end) { end -= 1; }
    &value[..end]
}

fn trim_ending(value: &str) -> &str { value.trim_end_matches([' ', '.']) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{FrontmatterField, SourceContainer, SourceRef};

    fn asset(provider: &str, resource: &str, canonical: &str, title: &str, instance: &str) -> SourceAsset {
        SourceAsset { source: SourceRef { provider_id: provider.into(), provider_instance: instance.into(), resource_type: resource.into(), canonical_id: canonical.into() }, title: title.into(), source_url: None, original_url: None, source_revision: None, complete: true, diagnostics: vec![], body: String::new(), container: None, fields: vec![], attachments: vec![] }
    }

    #[test]
    fn maps_each_supported_provider_resource() {
        let cases = [
            ("confluence", "page", "42", "Release Notes", "https://wiki.example/wiki", vec!["confluence", "wiki.example", "ENG - Engineering", "Roadmap"], "Release Notes", "Release Notes.md"),
            ("jira", "issue", "PROJ-123", "Fix login", "https://jira.example", vec!["jira", "jira.example", "PROJ"], "PROJ-123", "Fix login.md"),
            ("gitlab", "issue", "team/repo#12", "Issue", "https://gitlab.example/group", vec!["gitlab", "gitlab.example-group", "team", "repo", "issues"], "12", "Issue.md"),
            ("gitlab", "review", "team/repo!3", "Review", "https://gitlab.example", vec!["gitlab", "gitlab.example", "team", "repo", "merge-requests"], "3", "Review.md"),
            ("github", "issue", "owner/repo#12", "Issue", "https://github.com", vec!["github", "github.com", "owner", "repo", "issues"], "12", "Issue.md"),
            ("github", "review", "owner/repo!3", "Pull", "https://github.com", vec!["github", "github.com", "owner", "repo", "pulls"], "3", "Pull.md"),
            ("tea", "issue", "owner/repo#12", "Issue", "https://tea.example", vec!["tea", "tea.example", "owner", "repo", "issues"], "12", "Issue.md"),
            ("tea", "review", "owner/repo!3", "Pull", "https://tea.example", vec!["tea", "tea.example", "owner", "repo", "pulls"], "3", "Pull.md"),
            ("tea", "wiki", "owner/repo:Docs/Start", "Start", "https://tea.example", vec!["tea", "tea.example", "owner", "repo", "wiki", "Docs"], "Start", "Start.md"),
            ("other", "artifact", "abc", "Fallback", "https://host.example", vec!["other", "host.example", "artifact"], "abc", "Fallback.md"),
            ("other", "page", "x", "Unknown page", "https://host.example", vec!["other", "host.example", "page"], "x", "Unknown page.md"),
        ];
        for (provider, kind, id, title, instance, expected, leaf, document) in cases {
            let mut a = asset(provider, kind, id, title, instance);
            if provider == "confluence" {
                a.container = Some(SourceContainer { id: "ENG".into(), label: "ENG · Engineering".into() });
                a.fields = vec![FrontmatterField { key: "ancestors".into(), value: FrontmatterValue::Strings(vec!["Roadmap".into()]) }];
            }
            let got = source_placement(&a);
            assert_eq!(got.container, expected.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            assert_eq!(got.leaf, leaf);
            assert_eq!(got.document, document);
        }
        let mut alias = asset("wiki", "page", "42", "Release Notes", "https://wiki.example/wiki");
        alias.container = Some(SourceContainer { id: "ENG".into(), label: "ENG · Engineering".into() });
        alias.fields = vec![
            FrontmatterField { key: "space_key".into(), value: FrontmatterValue::String("ENG".into()) },
            FrontmatterField { key: "ancestors".into(), value: FrontmatterValue::Strings(vec!["Roadmap".into()]) },
        ];
        assert_eq!(
            source_placement(&alias).container,
            ["confluence", "wiki.example", "ENG - Engineering", "Roadmap"].map(str::to_owned).to_vec()
        );
    }

    #[test]
    fn sanitizes_unicode_controls_and_reserved_names() {
        assert_eq!(segment("  Café 世界 / ok\\bad\0\u{7f}"), "  Café 世界 - ok-bad--");
        assert_eq!(segment(".hidden"), "_hidden");
        assert_eq!(segment("... "), "_");
        assert_eq!(folder_leaf("文档 / Folder"), "文档 - Folder");
        assert!(is_reserved(".cockpit", false));
        assert!(is_reserved("_files", false));
        assert!(is_reserved(".", false));
        assert!(is_reserved("..", false));
        assert!(is_reserved("README.md", true));
        assert!(!is_reserved("README.md", false));
    }

    #[test]
    fn truncates_segments_and_tags_on_utf8_boundaries() {
        let long = format!("{}", "界".repeat(200));
        assert!(segment(&long).len() <= 255);
        let tagged_name = tagged(&long, "id");
        assert!(tagged_name.len() + 3 <= 255);
        assert!(tagged_name.is_char_boundary(tagged_name.len()));
        let huge_tag = tagged(&long, &"x".repeat(300));
        assert!(huge_tag.len() + 3 <= 255);
        assert_eq!(tagged_name, format!("{} [id]", truncate(&long, 255 - 5 - 3)));
        let a = asset("jira", "issue", "PROJ-123", &long, "https://jira.example");
        let p = source_placement(&a);
        assert!(p.document.len() <= 255);
        assert!(p.document.ends_with(".md"));
    }

    #[test]
    fn preserves_non_default_port_and_base_path() {
        let a = asset("gitlab", "issue", "org/repo#5", "Task", "https://git.example:8443/git");
        assert_eq!(source_placement(&a).container[1], "git.example_8443-git");
    }
}
