//! Pure helpers for Jira JQL follows: input recognition and normalization,
//! relative-date detection and the small amount of civil-date arithmetic the
//! windowed listing needs. No I/O and no date dependency.

use crate::InspectionError;
use crate::repositories::is_jira_key;

const MAX_JQL_BYTES: usize = 2048;

/// A recognized Jira query input. `jql` is normalized and never carries a
/// trailing `ORDER BY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JiraQueryInput {
    pub jql: String,
    /// The input was a bare project key, expanded to `project = KEY`.
    pub bare_project: bool,
}

fn unrecognized(message: &str) -> InspectionError {
    InspectionError::new("library_input_unrecognized", message)
}

fn bare_project_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    (2..=32).contains(&bytes.len())
        && bytes[0].is_ascii_uppercase()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
}

fn identifier_char(value: char) -> bool {
    value.is_alphanumeric() || value == '_'
}

/// The characters of `text` that lie outside quoted strings; quoted text and
/// its quotes become `'\0'` so positions stay aligned with `text`.
fn outside_quotes(text: &str) -> Vec<char> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    text.chars()
        .map(|character| match quote {
            Some(open) => {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == open {
                    quote = None;
                }
                '\0'
            }
            None if character == '"' || character == '\'' => {
                quote = Some(character);
                '\0'
            }
            None => character,
        })
        .collect()
}

fn has_operator(outside: &[char]) -> bool {
    if outside
        .iter()
        .any(|character| matches!(character, '=' | '~' | '<' | '>'))
    {
        return true;
    }
    let lowered: String = outside
        .iter()
        .map(|character| if *character == '\0' { ' ' } else { character.to_ascii_lowercase() })
        .collect();
    lowered
        .split_whitespace()
        .any(|word| matches!(word, "in" | "is" | "was" | "changed"))
        && lowered.split_whitespace().count() > 1
}

/// Trim, collapse whitespace outside quotes and validate quoting and
/// bracketing. Rejects anything that could close the caller's guard group.
fn normalize(text: &str) -> Result<String, InspectionError> {
    let mut out = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut depth = 0usize;
    let mut pending_space = false;
    for character in text.chars() {
        if let Some(open) = quote {
            if character.is_control() {
                return Err(unrecognized("Jira query contains a control character"));
            }
            out.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open {
                quote = None;
            }
            continue;
        }
        if character.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if character.is_control() {
            return Err(unrecognized("Jira query contains a control character"));
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '(' => depth += 1,
            ')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| unrecognized("Jira query has an unbalanced parenthesis"))?;
            }
            _ => {}
        }
        out.push(character);
    }
    if quote.is_some() {
        return Err(unrecognized("Jira query has an unbalanced quote"));
    }
    if depth != 0 {
        return Err(unrecognized("Jira query has an unbalanced parenthesis"));
    }
    Ok(out)
}

/// Cut a top-level (outside quotes and parentheses) `ORDER BY …` tail.
fn strip_order_by(text: &str) -> String {
    let outside = outside_quotes(text);
    let chars: Vec<char> = text.chars().collect();
    let mut depth = 0usize;
    for index in 0..chars.len() {
        match outside[index] {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            'o' | 'O' if depth == 0 => {
                let word: String = outside[index..]
                    .iter()
                    .take(8)
                    .map(|character| character.to_ascii_lowercase())
                    .collect();
                let boundary = index == 0 || !identifier_char(outside[index - 1]);
                if boundary
                    && word.starts_with("order")
                    && outside.get(index + 5).is_some_and(|c| c.is_whitespace())
                {
                    let rest: String = outside[index + 5..]
                        .iter()
                        .collect::<String>()
                        .trim_start()
                        .to_ascii_lowercase();
                    let after = rest.strip_prefix("by");
                    if after.is_some_and(|tail| {
                        tail.chars().next().is_none_or(|c| !identifier_char(c))
                    }) {
                        return chars[..index].iter().collect::<String>().trim_end().to_owned();
                    }
                }
            }
            _ => {}
        }
    }
    text.to_owned()
}

/// Whether `jql` is exactly its own normalization: trimmed, single-spaced
/// outside quotes, quotes and parentheses balanced (never closing more than
/// opened), no control characters. Anything else is unsafe to embed in a
/// guard group.
pub fn is_normalized_jql(jql: &str) -> bool {
    !jql.is_empty() && normalize(jql).is_ok_and(|normalized| normalized == jql)
}

/// Recognize a Jira query input. `None` means "not a Jira query": an
/// http(s) URL, a Jira key, a folder path or plain text without JQL
/// operators. A bare project key becomes `project = KEY`.
pub fn jira_query_input(input: &str) -> Result<Option<JiraQueryInput>, InspectionError> {
    let text = input.trim();
    let lowered = text.to_ascii_lowercase();
    if text.is_empty()
        || lowered.starts_with("http://")
        || lowered.starts_with("https://")
        || is_jira_key(text)
        || text.starts_with(['/', '~', '.'])
    {
        return Ok(None);
    }
    if bare_project_key(text) {
        return Ok(Some(JiraQueryInput {
            jql: format!("project = {text}"),
            bare_project: true,
        }));
    }
    if !has_operator(&outside_quotes(text)) {
        return Ok(None);
    }
    if text.len() > MAX_JQL_BYTES * 4 {
        return Err(unrecognized("Jira query is longer than 2048 bytes"));
    }
    let normalized = normalize(text)?;
    let jql = strip_order_by(&normalized);
    if jql.is_empty() {
        return Err(unrecognized("Jira query is empty"));
    }
    if jql.len() > MAX_JQL_BYTES {
        return Err(unrecognized("Jira query is longer than 2048 bytes"));
    }
    Ok(Some(JiraQueryInput {
        jql,
        bare_project: false,
    }))
}

/// Whether the query's meaning moves with time or login: `now()`,
/// `startOf…(`, `endOf…(`, `currentLogin(`, `lastLogin(` or a `-<n>[wdhm]`
/// offset, outside quotes.
pub fn has_relative_dates(jql: &str) -> bool {
    let outside: Vec<char> = outside_quotes(jql)
        .into_iter()
        .map(|character| if character == '\0' || !character.is_ascii() { ' ' } else { character.to_ascii_lowercase() })
        .collect();
    let text: String = outside.iter().collect();
    for (index, _) in text.char_indices() {
        let before = index
            .checked_sub(1)
            .and_then(|previous| outside.get(previous))
            .copied();
        if before.is_some_and(identifier_char) {
            continue;
        }
        let rest = &text[index..];
        if rest.starts_with("now(")
            || rest.starts_with("currentlogin(")
            || rest.starts_with("lastlogin(")
            || ["startof", "endof"].iter().any(|prefix| {
                rest.strip_prefix(prefix).is_some_and(|tail| {
                    let name = tail.split('(').next().unwrap_or_default();
                    tail.contains('(')
                        && !name.is_empty()
                        && name.chars().all(|character| character.is_ascii_alphabetic())
                })
            })
        {
            return true;
        }
        if let Some(tail) = rest.strip_prefix('-') {
            let digits = tail.chars().take_while(char::is_ascii_digit).count();
            let mut after = tail[digits..].chars();
            if digits > 0
                && after.next().is_some_and(|unit| matches!(unit, 'w' | 'd' | 'h' | 'm'))
                && after.next().is_none_or(|next| !identifier_char(next))
            {
                return true;
            }
        }
    }
    false
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn digits(bytes: &[u8]) -> Option<i64> {
    bytes
        .iter()
        .all(u8::is_ascii_digit)
        .then(|| bytes.iter().fold(0i64, |acc, byte| acc * 10 + i64::from(byte - b'0')))
}

/// `(days since epoch, hour, minute)` from a `YYYY-MM-DD[ T]HH:MM` prefix.
fn parse_wall(bytes: &[u8]) -> Option<(i64, i64, i64)> {
    if bytes.len() < 16
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b' ' | b'T')
        || bytes[13] != b':'
    {
        return None;
    }
    let (year, month, day) = (digits(&bytes[0..4])?, digits(&bytes[5..7])?, digits(&bytes[8..10])?);
    let (hour, minute) = (digits(&bytes[11..13])?, digits(&bytes[14..16])?);
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
    {
        return None;
    }
    Some((days_from_civil(year, month, day), hour, minute))
}

/// `YYYY-MM-DD[ T]HH:MM…` as minutes since 1970-01-01, ignoring any zone.
pub fn wall_minute(value: &str) -> Option<i64> {
    let (days, hour, minute) = parse_wall(value.as_bytes())?;
    Some(days * 1440 + hour * 60 + minute)
}

/// Minutes since 1970-01-01 as `YYYY-MM-DD HH:MM`.
pub fn format_wall_minute(minute: i64) -> String {
    let (year, month, day) = civil_from_days(minute.div_euclid(1440));
    let of_day = minute.rem_euclid(1440);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", of_day / 60, of_day % 60)
}

/// `…T…(.fff)?(Z|±HHMM|±HH:MM)` as UTC epoch seconds.
pub fn instant_seconds(iso: &str) -> Option<i64> {
    let bytes = iso.as_bytes();
    if bytes.len() < 20 || bytes[10] != b'T' || bytes[16] != b':' {
        return None;
    }
    let (days, hour, minute) = parse_wall(&bytes[..16])?;
    let second = digits(&bytes[17..19])?;
    if second > 59 {
        return None;
    }
    let mut rest = &bytes[19..];
    if rest.first() == Some(&b'.') {
        let count = rest[1..].iter().take_while(|byte| byte.is_ascii_digit()).count();
        if count == 0 {
            return None;
        }
        rest = &rest[1 + count..];
    }
    let offset = match rest {
        [b'Z'] => 0,
        [sign @ (b'+' | b'-'), zone @ ..] => {
            let (hours, minutes) = match zone {
                [_, _, _, _] => (digits(&zone[..2])?, digits(&zone[2..])?),
                [_, _, b':', _, _] => (digits(&zone[..2])?, digits(&zone[3..])?),
                _ => return None,
            };
            if hours > 23 || minutes > 59 {
                return None;
            }
            let magnitude = hours * 3600 + minutes * 60;
            if *sign == b'+' { magnitude } else { -magnitude }
        }
        _ => return None,
    };
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jql(input: &str) -> String {
        jira_query_input(input).unwrap().unwrap().jql
    }

    #[test]
    fn expands_bare_project_keys_with_digits_and_underscores() {
        let longest = "A".repeat(32);
        for project in ["AB", "OPS", "AB1", "AB_1", "A_", longest.as_str()] {
            assert_eq!(
                jira_query_input(project).unwrap(),
                Some(JiraQueryInput {
                    jql: format!("project = {project}"),
                    bare_project: true,
                }),
                "{project}"
            );
        }
    }

    #[test]
    fn leaves_non_project_tokens_unrecognized() {
        let too_long = "A".repeat(33);
        for input in ["A", "1A", "_A", "aB", "Ab", "AB!", "AB-XY", "ABé", too_long.as_str()] {
            assert_eq!(jira_query_input(input).unwrap(), None, "{input}");
        }
    }

    #[test]
    fn recognizes_uppercase_jql_without_expanding_it_as_a_project() {
        assert_eq!(
            jira_query_input("PROJECT = OPS").unwrap(),
            Some(JiraQueryInput { jql: "PROJECT = OPS".into(), bare_project: false })
        );
    }

    #[test]
    fn normalizes_queries_and_leaves_other_inputs_alone() {
        assert_eq!(jira_query_input("OPS-12").unwrap(), None);
        assert_eq!(jira_query_input("https://x.atlassian.net/browse/OPS-1?a=b").unwrap(), None);
        assert_eq!(jira_query_input("/tmp/a=b").unwrap(), None);
        assert_eq!(jira_query_input("just some words").unwrap(), None);
        assert_eq!(
            jql("  project =  OPS \n AND summary ~ \"a   b\"  ORDER  BY updated DESC "),
            "project = OPS AND summary ~ \"a   b\""
        );
        // `order by` inside quotes or a subquery is not the trailing clause.
        assert_eq!(
            jql("summary ~ \"x order by y\""),
            "summary ~ \"x order by y\""
        );
        assert_eq!(
            jql("assignee in (a, b) and status = 'Done' order by key"),
            "assignee in (a, b) and status = 'Done'"
        );
        assert_eq!(jql("labels is EMPTY"), "labels is EMPTY");
    }

    #[test]
    fn rejects_queries_that_could_escape_the_guard_group() {
        for bad in [
            "project = A) OR (project = B",
            "project = \"A",
            "(project = A",
            "project = A\u{7}",
            "order by key = 1",
        ] {
            let error = jira_query_input(bad).unwrap_err();
            assert_eq!(error.code, "library_input_unrecognized", "{bad}");
        }
        let long = format!("summary ~ \"{}\"", "x".repeat(2100));
        assert!(jira_query_input(&long).is_err());
    }

    #[test]
    fn relative_dates_are_found_outside_quotes_only() {
        for relative in [
            "updated >= -14d",
            "updated>=-1w",
            "created > startOfMonth()",
            "due < endOfDay(\"+1d\")",
            "updated > now()",
            "assignee = currentLogin()",
            "updated >= -30m AND x = 1",
        ] {
            assert!(has_relative_dates(relative), "{relative}");
        }
        for fixed in [
            "project = OPS",
            "summary ~ \"-7d now()\"",
            "updated >= \"2026-01-01\"",
            "key = ABC-1",
            "assignee = currentUser()",
            "summary ~ known(x)",
        ] {
            assert!(!has_relative_dates(fixed), "{fixed}");
        }
    }

    #[test]
    fn wall_minutes_cross_day_month_and_leap_boundaries() {
        let end_of_month = wall_minute("2024-02-29 23:59:59").unwrap();
        assert_eq!(format_wall_minute(end_of_month + 1), "2024-03-01 00:00");
        assert_eq!(format_wall_minute(end_of_month), "2024-02-29 23:59");
        let start = wall_minute("2026-01-01T00:00:00.000+0100").unwrap();
        assert_eq!(format_wall_minute(start - 1), "2025-12-31 23:59");
        assert_eq!(wall_minute("1970-01-01 00:00"), Some(0));
        assert_eq!(format_wall_minute(-1), "1969-12-31 23:59");
        assert_eq!(wall_minute("2026-02-30 10:00"), None);
        assert_eq!(wall_minute("2026-01-01"), None);
    }

    #[test]
    fn instants_apply_the_offset() {
        assert_eq!(instant_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(instant_seconds("1970-01-01T02:00:00.000+0200"), Some(0));
        assert_eq!(instant_seconds("1970-01-01T02:00:00+02:00"), Some(0));
        assert_eq!(instant_seconds("1969-12-31T19:30:01.5-0430"), Some(1));
        assert_eq!(instant_seconds("2026-09-28 11:28:44"), None);
        assert_eq!(instant_seconds("2026-09-28T11:28:44"), None);
    }
}
