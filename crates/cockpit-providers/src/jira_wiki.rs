//! Jira wiki markup to Markdown, for self-hosted Jira that returns wiki
//! strings instead of ADF.
//!
//! Conservative by design: block syntax converts only at line start, emphasis
//! only at word boundaries, and code, monospace, links and bare URLs are
//! lifted out into opaque tokens first so nothing inside them is rewritten.

const OPEN: char = '\u{E000}';
const CLOSE: char = '\u{E001}';

/// Protected spans, referenced from the text as `OPEN index CLOSE`.
#[derive(Default)]
struct Vault(Vec<String>);

impl Vault {
    fn put(&mut self, value: String) -> String {
        self.0.push(value);
        format!("{OPEN}{}{CLOSE}", self.0.len() - 1)
    }

    /// Put the tokens back. A multi-line value (a fence) that starts a
    /// quoted line keeps that line's `> ` prefix on every line.
    fn expand(&self, line: &str) -> String {
        let mut out = String::new();
        let mut rest = line;
        while let Some(start) = rest.find(OPEN) {
            out.push_str(&rest[..start]);
            let after = &rest[start + OPEN.len_utf8()..];
            let Some((index, tail)) = after.split_once(CLOSE) else {
                rest = after;
                continue;
            };
            let value = index.parse::<usize>().ok().and_then(|index| self.0.get(index));
            if let Some(value) = value {
                let value = self.expand(value);
                if out.chars().all(|c| c == ' ' || c == '>') {
                    out.push_str(&value.replace('\n', &format!("\n{out}")));
                } else {
                    out.push_str(&value);
                }
            }
            rest = tail;
        }
        out.push_str(rest);
        out
    }
}

/// Convert Jira wiki markup to Markdown. `heading_offset` shifts `hN.`
/// headings so they nest under the caller's own headings (capped at `######`).
pub fn wiki_to_markdown(input: &str, heading_offset: usize) -> String {
    let mut vault = Vault::default();
    let text: String = input
        .replace("\r\n", "\n")
        .chars()
        .filter(|c| *c != OPEN && *c != CLOSE)
        .collect();
    let text = protect_code(&text, &mut vault);
    let text = strip_tags(&text);
    let text = text
        .split('\n')
        .map(|line| protect_line(line, &mut vault))
        .collect::<Vec<_>>()
        .join("\n");
    let out = blocks(&text, heading_offset);
    out.split('\n')
        .map(|line| vault.expand(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn fence(content: &str, language: &str) -> String {
    let ticks = "`".repeat(longest_run(content).max(2) + 1);
    let content = content.strip_prefix('\n').unwrap_or(content);
    let content = content.strip_suffix('\n').unwrap_or(content);
    format!("{ticks}{language}\n{content}\n{ticks}")
}

fn inline_code(content: &str) -> String {
    let ticks = "`".repeat(longest_run(content) + 1);
    let pad = if content.starts_with('`') || content.ends_with('`') { " " } else { "" };
    format!("{ticks}{pad}{content}{pad}{ticks}")
}

fn longest_run(text: &str) -> usize {
    text.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

/// `{code}`, `{code:lang}` and `{noformat}` become fences (or inline code
/// when they sit mid-sentence on one line). An unclosed opener stays text.
fn protect_code(text: &str, vault: &mut Vault) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let tag = ["code", "noformat"].into_iter().find(|tag| {
            rest[1..].strip_prefix(tag).is_some_and(|after| after.starts_with(['}', ':']))
        });
        let close = tag.and_then(|tag| Some((tag, rest.find('}')?)));
        let body = close.and_then(|(tag, open_end)| {
            let closer = format!("{{{tag}}}");
            let length = rest[open_end + 1..].find(&closer)?;
            Some((tag, open_end, open_end + 1 + length, closer.len()))
        });
        let Some((tag, open_end, body_end, closer_len)) = body.filter(|(_, open_end, ..)| !rest[..*open_end].contains('\n')) else {
            out.push('{');
            rest = &rest[1..];
            continue;
        };
        let content = &rest[open_end + 1..body_end];
        let before_blank = out.rsplit('\n').next().is_some_and(|line| line.trim().is_empty());
        let after = &rest[body_end + closer_len..];
        let after_blank = after.split('\n').next().is_some_and(|line| line.trim().is_empty());
        if before_blank && after_blank || content.contains('\n') {
            let language = if tag == "code" { code_language(&rest[tag.len() + 1..open_end]) } else { "" };
            out.push_str("\n\n");
            out.push_str(&vault.put(fence(content, language)));
            out.push_str("\n\n");
        } else {
            out.push_str(&vault.put(inline_code(content)));
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The language from `{code:java}` or `{code:title=x|java}`; anything that
/// is not a plain language name is dropped.
fn code_language(params: &str) -> &str {
    params
        .strip_prefix(':')
        .unwrap_or_default()
        .split('|')
        .map(|param| param.strip_prefix("language=").unwrap_or(param))
        .find(|param| {
            !param.is_empty()
                && param.chars().all(|c| c.is_ascii_alphanumeric() || "_+#.-".contains(c))
        })
        .unwrap_or_default()
}

/// Drop `{color}` tags and `{panel}` markers (a panel title becomes bold).
fn strip_tags(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let name = ["color", "panel"]
            .into_iter()
            .find(|name| rest[1..].strip_prefix(name).is_some_and(|after| after.starts_with(['}', ':'])));
        match (name, rest.find('}')) {
            (Some(name), Some(end)) if !rest[..end].contains('\n') => {
                if name == "panel" {
                    out.push_str("\n\n");
                    let title = rest[..end]
                        .split(['|', ':'])
                        .find_map(|param| param.strip_prefix("title="))
                        .filter(|title| !title.trim().is_empty());
                    if let Some(title) = title {
                        out.push_str(&format!("**{}**\n\n", title.trim()));
                    }
                }
                rest = &rest[end + 1..];
            }
            _ => {
                out.push('{');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Lift monospace, images, links and bare URLs out of one line.
fn protect_line(line: &str, vault: &mut Vault) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some((value, used)) = inline_span(&chars, i) {
            out.push_str(&vault.put(value));
            i += used;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn inline_span(chars: &[char], i: usize) -> Option<(String, usize)> {
    let after_word = i > 0 && chars[i - 1].is_alphanumeric();
    let find = |from: usize, needle: &[char]| {
        chars[from..].windows(needle.len()).position(|window| window == needle).map(|at| from + at)
    };
    match chars[i] {
        '{' if chars.get(i + 1) == Some(&'{') => {
            let end = find(i + 2, &['}', '}']).filter(|end| *end > i + 2)?;
            Some((inline_code(&chars[i + 2..end].iter().collect::<String>()), end + 2 - i))
        }
        '!' if !after_word => {
            let end = find(i + 1, &['!'])?;
            let inner: String = chars[i + 1..end].iter().collect();
            let target = inner.split('|').next().unwrap_or_default();
            if target.is_empty() || target.contains(char::is_whitespace) {
                return None;
            }
            let image = if target.contains("://") {
                format!("![]({})", encode_url(target))
            } else {
                format!("[image: {target}]")
            };
            Some((image, end + 1 - i))
        }
        '[' => {
            let end = i + 1 + chars[i + 1..].iter().position(|c| matches!(c, ']' | '['))?;
            (chars[end] == ']').then_some(())?;
            let inner: String = chars[i + 1..end].iter().collect();
            Some((link(&inner)?, end + 1 - i))
        }
        'h' if !after_word => {
            let starts = |prefix: &str| chars[i..].iter().take(prefix.len()).copied().eq(prefix.chars());
            if !(starts("http://") || starts("https://")) {
                return None;
            }
            let end = chars[i..]
                .iter()
                .position(|c| c.is_whitespace() || "<>\"[]|".contains(*c))
                .map_or(chars.len(), |at| i + at);
            let url: String = chars[i..end].iter().collect();
            let mut url = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_', '+', '-', '~', '\'']);
            if !url.contains('(') {
                url = url.trim_end_matches(')');
            }
            let length = url.chars().count();
            (length > "https://".len()).then(|| (url.to_owned(), length))
        }
        _ => None,
    }
}

fn link(inner: &str) -> Option<String> {
    if let Some(user) = inner.strip_prefix('~') {
        return (!user.is_empty() && !user.contains(char::is_whitespace)).then(|| format!("@{user}"));
    }
    let (text, target, piped) = match inner.split_once('|') {
        Some((text, rest)) => (text.trim(), rest.split('|').next().unwrap_or_default().trim(), true),
        None => ("", inner.trim(), false),
    };
    let is_url = !target.contains(char::is_whitespace)
        && (target.contains("://")
            || target.starts_with("mailto:")
            || (piped && target.starts_with(['/', '#'])));
    if is_url {
        return Some(if text.is_empty() {
            format!("<{target}>")
        } else {
            format!("[{}]({})", emphasis(text), encode_url(target))
        });
    }
    let attachment = target.strip_prefix('^').filter(|name| !name.is_empty())?;
    Some(if text.is_empty() { inline_code(attachment) } else { emphasis(text) })
}

fn encode_url(url: &str) -> String {
    url.replace('(', "%28").replace(')', "%29")
}

/// `*bold*` -> `**bold**`, `-strike-` -> `~~strike~~`, `+underline+` -> plain.
/// `_italic_` is already Markdown and is left alone.
fn emphasis(text: &str) -> String {
    let text = pair(text, '*', "**");
    let text = pair(&text, '-', "~~");
    pair(&text, '+', "")
}

fn pair(text: &str, marker: char, wrap: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let strict = marker != '*';
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == marker && opens(&c, i, marker, strict) {
            let close = (i + 2..c.len()).find(|&j| {
                c[j] == marker
                    && !c[j - 1].is_whitespace()
                    && c[j - 1] != marker
                    && c.get(j + 1).is_none_or(|next| !next.is_alphanumeric() && *next != marker)
            });
            if let Some(close) = close {
                out.push_str(wrap);
                out.extend(&c[i + 1..close]);
                out.push_str(wrap);
                i = close + 1;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

fn opens(c: &[char], i: usize, marker: char, strict: bool) -> bool {
    let boundary = i == 0 || (!c[i - 1].is_alphanumeric() && c[i - 1] != marker);
    let next = c.get(i + 1).copied();
    boundary
        && next.is_some_and(|next| {
            !next.is_whitespace()
                && next != marker
                && (!strict || next.is_alphanumeric() || next == OPEN)
        })
}

/// Block structure. `{quote}` pairs nest recursively; an unpaired one is text.
fn blocks(text: &str, offset: usize) -> String {
    let pieces: Vec<&str> = text.split("{quote}").collect();
    let mut parts = Vec::new();
    let mut plain = String::new();
    for (n, piece) in pieces.iter().enumerate() {
        if n % 2 == 1 && n + 1 < pieces.len() {
            parts.push(lines(&plain, offset));
            plain.clear();
            let quoted = blocks(piece, offset);
            let prefixed: Vec<String> = quoted
                .split('\n')
                .map(|line| if line.is_empty() { ">".into() } else { format!("> {line}") })
                .collect();
            parts.push(prefixed.join("\n"));
        } else {
            if n % 2 == 1 {
                plain.push_str("{quote}");
            }
            plain.push_str(piece);
        }
    }
    parts.push(lines(&plain, offset));
    parts.retain(|part| !part.trim().is_empty());
    parts.join("\n\n")
}

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Blank,
    Paragraph,
    List,
    Quote,
    Table,
    Single,
}

fn lines(text: &str, offset: usize) -> String {
    let classified: Vec<(Kind, String)> = text.split('\n').map(|line| classify(line, offset)).collect();
    let mut out: Vec<String> = Vec::new();
    let mut group: Vec<&str> = Vec::new();
    let mut kind = Kind::Blank;
    let mut flush = |kind: Kind, group: &mut Vec<&str>| {
        if kind == Kind::Table {
            out.push(table(group));
        } else if !group.is_empty() {
            out.push(group.join("\n"));
        }
        group.clear();
    };
    for (line_kind, line) in &classified {
        if *line_kind != kind || *line_kind == Kind::Single {
            flush(kind, &mut group);
            kind = *line_kind;
        }
        if *line_kind != Kind::Blank {
            group.push(line);
        }
    }
    flush(kind, &mut group);
    out.join("\n\n")
}

fn classify(line: &str, offset: usize) -> (Kind, String) {
    let line = line.trim_end();
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return (Kind::Blank, String::new());
    }
    if trimmed == "----" {
        return (Kind::Single, "---".into());
    }
    let mut chars = trimmed.chars();
    if let (Some('h'), Some(level @ '1'..='6'), Some('.'), Some(space)) =
        (chars.next(), chars.next(), chars.next(), chars.next())
        && space.is_whitespace()
        && !trimmed[3..].trim().is_empty()
    {
        let level = level.to_digit(10).unwrap_or(1) as usize;
        let hashes = "#".repeat((level + offset).min(6));
        return (Kind::Single, format!("{hashes} {}", emphasis(trimmed[3..].trim())));
    }
    if let Some(quote) = trimmed.strip_prefix("bq.").filter(|rest| rest.starts_with(' ')) {
        return (Kind::Quote, format!("> {}", emphasis(quote.trim())));
    }
    let marks = trimmed.chars().take_while(|c| matches!(c, '*' | '#' | '-')).count();
    let (marks, rest) = trimmed.split_at(marks);
    if !marks.is_empty()
        && (marks == "-" || marks.chars().all(|c| c != '-'))
        && rest.starts_with([' ', '\t'])
        && !rest.trim().is_empty()
    {
        let indent: usize = marks[..marks.len() - 1].chars().map(|c| if c == '#' { 3 } else { 2 }).sum();
        let marker = if marks.ends_with('#') { "1." } else { "-" };
        return (Kind::List, format!("{}{marker} {}", " ".repeat(indent), emphasis(rest.trim())));
    }
    if trimmed.len() > 1 && trimmed.starts_with('|') && trimmed.ends_with('|') {
        return (Kind::Table, trimmed.into());
    }
    (Kind::Paragraph, emphasis(trimmed))
}

/// `||head||` rows are headers, `|cell|` rows are body; a table with no
/// header row gets an empty one because GFM requires it.
fn table(rows: &[&str]) -> String {
    let parsed: Vec<(bool, Vec<String>)> = rows
        .iter()
        .map(|row| {
            let cells = row
                .split('|')
                .filter(|cell| !cell.is_empty())
                .map(|cell| emphasis(cell.trim()).replace('|', "\\|"))
                .collect();
            (row.starts_with("||"), cells)
        })
        .collect();
    let width = parsed.iter().map(|(_, cells)| cells.len()).max().unwrap_or(1);
    let render = |cells: &[String]| {
        let padded: Vec<&str> = (0..width).map(|n| cells.get(n).map_or("", String::as_str)).collect();
        format!("|{}", padded.iter().map(|cell| format!(" {cell} |")).collect::<String>())
    };
    let header_first = parsed.first().is_some_and(|(header, _)| *header);
    let mut out = Vec::new();
    out.push(if header_first { render(&parsed[0].1) } else { render(&[]) });
    out.push(format!("|{}", " --- |".repeat(width)));
    out.extend(parsed.iter().skip(usize::from(header_first)).map(|(_, cells)| render(cells)));
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::wiki_to_markdown;

    #[test]
    fn converts_wiki_constructs_and_leaves_lookalikes_alone() {
        let cases = [
            ("h1. Title\nh3. Sub *b*", "# Title\n\n### Sub **b**"),
            ("h2.NoSpace and h7. nope", "h2.NoSpace and h7. nope"),
            (
                "*bold* _it_ -gone- +under+ {{mono_x*y}}",
                "**bold** _it_ ~~gone~~ under `mono_x*y`",
            ),
            ("{code:java}\nfn a_b() { *x* h1. y }\n{code}", "```java\nfn a_b() { *x* h1. y }\n```"),
            ("{code:title=T|rust}\nlet x;\n{code}", "```rust\nlet x;\n```"),
            ("{noformat}\n*raw* {{x}}\n{noformat}", "```\n*raw* {{x}}\n```"),
            ("{code}unchanged{code}", "```\nunchanged\n```"),
            ("use {code}x*y{code} inline", "use `x*y` inline"),
            ("{code}unclosed *b*", "{code}unclosed **b**"),
            ("{quote}\nquoted *b*\n{quote}\nafter", "> quoted **b**\n\nafter"),
            ("{quote}Legacy comment{quote}", "> Legacy comment"),
            ("{quote}\n{code}\nx\n{code}\n{quote}", "> ```\n> x\n> ```"),
            ("bq. hi *there*", "> hi **there**"),
            (
                "* a\n** b\n# one\n## two\n- x\n\npara",
                "- a\n  - b\n1. one\n   1. two\n- x\n\npara",
            ),
            (
                "[docs|https://e.test/a_b] [https://e.test] [~jdoe] [PROJ-1] [i] !pic.png|width=9! !https://e.test/p.png!",
                "[docs](https://e.test/a_b) <https://e.test> @jdoe [PROJ-1] [i] [image: pic.png] ![](https://e.test/p.png)",
            ),
            (
                "||a||b||\n|1|2|\n|x|[l|https://e.test]|",
                "| a | b |\n| --- | --- |\n| 1 | 2 |\n| x | [l](https://e.test) |",
            ),
            ("|1|2|\n|3|", "|  |  |\n| --- | --- |\n| 1 | 2 |\n| 3 |  |"),
            ("a\n----\nb", "a\n\n---\n\nb"),
            (
                "{panel:title=Note|borderStyle=solid}\n{color:red}careful{color}\n{panel}",
                "**Note**\n\ncareful",
            ),
            (
                "a*b*c snake_case_name 2*3 https://e.test/a_b_/x*y* 5 - 3 - 1 c++ and c++ wow!ok!",
                "a*b*c snake_case_name 2*3 https://e.test/a_b_/x*y* 5 - 3 - 1 c++ and c++ wow!ok!",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(wiki_to_markdown(input, 0), expected, "input: {input:?}");
        }
        assert_eq!(wiki_to_markdown("h1. A\nh5. B", 2), "### A\n\n###### B");
    }
}
