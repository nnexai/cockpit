use super::*;

pub(super) fn parse_issue_details(
    value: &Value,
    title: String,
    web_url: String,
) -> Result<IssueFacts, InspectionError> {
    let author = value
        .get("author")
        .and_then(|author| author.get("username"))
        .and_then(Value::as_str)
        .ok_or_else(|| contract_error("GitLab issue has no author username"))?
        .to_owned();
    let state = required_string(&value, "state", "GitLab issue")?;
    required_string_array(&value, "labels", "GitLab issue")?;
    let mut assignees = value
        .get("assignees")
        .and_then(Value::as_array)
        .ok_or_else(|| contract_error("GitLab issue has no assignees array"))?
        .iter()
        .map(|assignee| {
            assignee
                .get("username")
                .and_then(Value::as_str)
                .ok_or_else(|| contract_error("GitLab issue assignee has no username"))
                .map(str::to_owned)
        })
        .collect::<Result<Vec<_>, _>>()?;
    assignees.sort();
    assignees.dedup();
    match value.get("milestone") {
        Some(Value::Null) => {}
        Some(milestone) => {
            milestone
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| contract_error("GitLab issue milestone has no title"))?;
        }
        None => return Err(contract_error("GitLab issue has no milestone field")),
    }
    let created_at = required_string(&value, "created_at", "GitLab issue")?;
    let updated_at = required_string(&value, "updated_at", "GitLab issue")?;
    let description = match value.get("description") {
        Some(Value::Null) => String::new(),
        Some(Value::String(description)) => description.clone(),
        Some(_) => {
            return Err(contract_error(
                "GitLab issue description has an invalid type",
            ));
        }
        None => return Err(contract_error("GitLab issue has no description field")),
    };
    Ok(IssueFacts {
        title,
        description,
        author,
        state,
        assignees,
        created_at,
        updated_at,
        web_url,
    })
}

pub(super) struct ReviewDetails {
    pub(super) title: String,
    pub(super) description: String,
    pub(super) state: String,
    pub(super) author: String,
    pub(super) labels: Vec<String>,
    pub(super) assignees: Vec<String>,
    pub(super) reviewers: Vec<String>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) web_url: String,
}

pub(super) fn parse_review_refs(
    value: &Value,
    project: &ProjectFacts,
) -> Result<(String, Option<String>, Option<String>), InspectionError> {
    let target_branch = required_string(&value, "target_branch", "GitLab merge request")?;
    if !valid_branch(&target_branch) {
        return Err(contract_error(
            "GitLab merge request target branch is invalid",
        ));
    }
    let source_branch = optional_string(&value, "source_branch", "GitLab merge request")?;
    if source_branch
        .as_deref()
        .is_some_and(|branch| !valid_branch(branch))
    {
        return Err(contract_error(
            "GitLab merge request source branch is invalid",
        ));
    }
    let head_sha = value
        .get("sha")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("diff_refs")
                .and_then(|refs| refs.get("head_sha"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned);
    if let Some(sha) = &head_sha {
        if !valid_commit(sha) {
            return Err(contract_error(
                "GitLab merge request head commit is invalid",
            ));
        }
    }
    let target_project_id = value_u64(&value, "target_project_id")
        .or_else(|| value_u64(&value, "project_id"))
        .ok_or_else(|| contract_error("GitLab merge request has no target project ID"))?;
    if target_project_id != project.id {
        return Err(identity_error(
            "GitLab merge request target project mismatches requested project",
        ));
    }
    Ok((target_branch, source_branch, head_sha))
}

pub(super) fn parse_discussion(
    discussion: &Value,
    review_url: &str,
    notes: &mut Vec<ReviewNote>,
) -> Result<(), InspectionError> {
    let discussion_id = required_string(&discussion, "id", "GitLab discussion")?;
    let raw_notes = discussion
        .get("notes")
        .and_then(Value::as_array)
        .ok_or_else(|| contract_error("GitLab discussion has no notes array"))?;
    for note in raw_notes {
        let id = value_u64(note, "id")
            .ok_or_else(|| contract_error("GitLab discussion note has no numeric ID"))?;
        let body = required_string(note, "body", "GitLab discussion note")?;
        let author = note
            .get("author")
            .and_then(|author| author.get("username"))
            .and_then(Value::as_str)
            .ok_or_else(|| contract_error("GitLab discussion note has no author username"))?
            .to_owned();
        let created_at = required_string(note, "created_at", "GitLab discussion note")?;
        let updated_at = required_string(note, "updated_at", "GitLab discussion note")?;
        let url = comment_url(review_url, note, id)?;
        let position = note
            .get("position")
            .map(position_text)
            .transpose()?
            .unwrap_or_else(|| "unsupported local review anchor".into());
        notes.push(ReviewNote {
            discussion_id: discussion_id.clone(),
            id,
            author,
            created_at,
            updated_at,
            url,
            body,
            position,
        });
    }
    Ok(())
}

pub(super) fn parse_comment(
    value: &Value,
    issue_url: &str,
) -> Result<CommentFacts, InspectionError> {
    let id = value_u64(&value, "id")
        .ok_or_else(|| contract_error("GitLab issue comment has no numeric ID"))?;
    let body = required_string(&value, "body", "GitLab issue comment")?;
    let author = value
        .get("author")
        .and_then(|author| author.get("username"))
        .and_then(Value::as_str)
        .ok_or_else(|| contract_error("GitLab issue comment has no author username"))?
        .to_owned();
    let created_at = required_string(&value, "created_at", "GitLab issue comment")?;
    let updated_at = required_string(&value, "updated_at", "GitLab issue comment")?;
    let url = comment_url(issue_url, &value, id)?;
    Ok(CommentFacts {
        id,
        author,
        created_at,
        updated_at,
        url,
        body,
    })
}

pub(super) fn review_diagnostics(
    request: &SourceFetchRequest,
    review: &ReviewFacts,
    discussions_complete: bool,
    approvals_complete: bool,
) -> Vec<ProjectDiagnostic> {
    let mut diagnostics = Vec::new();
    if review.source_branch.as_deref().map_or(true, str::is_empty) {
        diagnostics.push(review_diagnostic(
            "source_branch_unavailable",
            "GitLab merge request source branch is unavailable",
            &request.artifact_url,
        ));
    }
    if review
        .head_sha
        .as_deref()
        .map_or(true, |sha| !valid_commit(sha))
    {
        diagnostics.push(review_diagnostic(
            "source_commit_unavailable",
            "GitLab merge request source commit is unavailable",
            &request.artifact_url,
        ));
    }
    if review.source_project.is_none() {
        diagnostics.push(review_diagnostic(
            "source_project_unavailable",
            "GitLab merge request source project provenance is unavailable",
            &request.artifact_url,
        ));
    }
    if !discussions_complete {
        diagnostics.push(review_diagnostic(
            "source_discussions_incomplete",
            "GitLab merge request discussions are unavailable or bounded",
            &request.artifact_url,
        ));
    }
    if !approvals_complete {
        diagnostics.push(review_diagnostic(
            "source_approvals_unavailable",
            "GitLab merge request approvals are unavailable or bounded",
            &request.artifact_url,
        ));
    }
    diagnostics
}

pub(super) fn append_review_description(
    request: &SourceFetchRequest,
    review: &ReviewFacts,
    budget: &mut ByteBudget,
    body: &mut String,
    diagnostics: &mut Vec<ProjectDiagnostic>,
) {
    let summary = summary_line(
        "Merge request",
        &review.state,
        &review.author,
        &review.assignees,
    );
    let header = format!("{}\n", summary);
    if !budget.append(body, &header) {
        diagnostics.push(review_diagnostic(
            "source_review_truncated",
            "GitLab merge request header exceeded Cockpit's explicit byte budget",
            &request.artifact_url,
        ));
    }
    if !review.description.trim().is_empty() {
        let description = format!(
            "\n## Description\n\n{}",
            demote_headings(&review.description, 3)
        );
        if !budget.append(body, &description) {
            diagnostics.push(review_diagnostic(
                "source_review_truncated",
                "GitLab merge request description exceeded Cockpit's explicit byte budget",
                &request.artifact_url,
            ));
        }
    }
}

pub(super) fn append_review_notes(
    request: &SourceFetchRequest,
    notes: &[ReviewNote],
    discussions_complete: bool,
    budget: &mut ByteBudget,
    body: &mut String,
    diagnostics: &mut Vec<ProjectDiagnostic>,
) {
    let rendered_notes: Vec<String> = notes
        .iter()
        .map(|note| {
            let edited = edited_timestamp(&note.created_at, &note.updated_at);
            let extra = review_location(&note.position);
            format!(
                "\n\n### {} · {}{}{}\n[#{id}]({url})\n\n{body}",
                note.author,
                local_timestamp(&note.created_at),
                edited.as_deref().unwrap_or(""),
                extra.as_deref().unwrap_or(""),
                id = note.id,
                url = note.url,
                body = demote_headings(&note.body, 4)
            )
        })
        .collect();
    let shown = comments_that_fit(
        budget.limit.saturating_sub(budget.used),
        &rendered_notes,
        discussions_complete,
    );
    if !notes.is_empty() {
        let comment_heading = comments_heading(shown, notes.len(), discussions_complete);
        if !budget.append(body, &comment_heading) {
            diagnostics.push(review_diagnostic(
                "source_review_truncated",
                "GitLab merge request discussions exceeded Cockpit's explicit byte budget",
                &request.artifact_url,
            ));
        }
        if shown < notes.len() {
            diagnostics.push(review_diagnostic(
                "source_review_truncated",
                "GitLab merge request discussions exceeded Cockpit's explicit byte budget",
                &request.artifact_url,
            ));
        }
        for rendered in rendered_notes.iter().take(shown) {
            if !budget.append(body, rendered) {
                diagnostics.push(review_diagnostic(
                    "source_review_truncated",
                    "GitLab merge request discussions exceeded Cockpit's explicit byte budget",
                    &request.artifact_url,
                ));
                break;
            }
        }
    }
}

pub(super) fn append_review_approvals(
    request: &SourceFetchRequest,
    approvals: Option<&ApprovalFacts>,
    budget: &mut ByteBudget,
    body: &mut String,
    diagnostics: &mut Vec<ProjectDiagnostic>,
) {
    if let Some(approvals) = approvals {
        let approved_by = approvals.approved_by.as_deref().unwrap_or(&[]).join(", ");
        let rendered = format!(
            "\n\n## Approvals\nApproved: {}\nApprovals left: {}\nApproved by: {}\n",
            approvals
                .approved
                .map(|value| value.to_string())
                .unwrap_or_else(|| "(unknown)".into()),
            approvals
                .approvals_left
                .map(|value| value.to_string())
                .unwrap_or_else(|| "(unknown)".into()),
            if approved_by.is_empty() {
                "(none)"
            } else {
                &approved_by
            },
        );
        if !budget.append(body, &rendered) {
            diagnostics.push(review_diagnostic(
                "source_review_truncated",
                "GitLab merge request approvals exceeded Cockpit's explicit byte budget",
                &request.artifact_url,
            ));
        }
    }
}

pub(super) fn render_issue(
    request: &SourceFetchRequest,
    issue: &IssueFacts,
    comments: &[CommentFacts],
    comments_complete: bool,
    budget: &mut ByteBudget,
) -> (String, Vec<ProjectDiagnostic>) {
    let mut diagnostics = Vec::new();
    let summary = summary_line("Issue", &issue.state, &issue.author, &issue.assignees);
    let mut body = String::new();
    let header = format!("{}\n", summary);
    if !budget.append(&mut body, &header) {
        diagnostics.push(truncation_diagnostic(&request.artifact_url));
    }
    if !issue.description.trim().is_empty() {
        let description = format!(
            "\n## Description\n\n{}",
            demote_headings(&issue.description, 3)
        );
        if !budget.append(&mut body, &description) {
            diagnostics.push(truncation_diagnostic(&request.artifact_url));
        }
    }
    let rendered_comments: Vec<String> = comments
        .iter()
        .map(|comment| {
            let edited = edited_timestamp(&comment.created_at, &comment.updated_at);
            format!(
                "\n\n### {} · {}{}\n[#{id}]({url})\n\n{body}",
                comment.author,
                local_timestamp(&comment.created_at),
                edited.as_deref().unwrap_or(""),
                id = comment.id,
                url = comment.url,
                body = demote_headings(&comment.body, 4)
            )
        })
        .collect();
    let shown = comments_that_fit(
        budget.limit.saturating_sub(budget.used),
        &rendered_comments,
        comments_complete,
    );
    if !comments.is_empty() {
        let comment_heading = comments_heading(shown, comments.len(), comments_complete);
        if !budget.append(&mut body, &comment_heading) {
            diagnostics.push(truncation_diagnostic(&request.artifact_url));
        }
        if shown < comments.len() {
            diagnostics.push(truncation_diagnostic(&request.artifact_url));
        }
        for rendered in rendered_comments.iter().take(shown) {
            if !budget.append(&mut body, rendered) {
                diagnostics.push(truncation_diagnostic(&request.artifact_url));
                break;
            }
        }
    }
    if shown < comments.len() {
        diagnostics.push(truncation_diagnostic(&request.artifact_url));
    }
    for rendered in rendered_comments.iter().take(shown) {
        if !budget.append(&mut body, rendered) {
            diagnostics.push(truncation_diagnostic(&request.artifact_url));
            break;
        }
    }
    if !comments_complete {
        diagnostics.push(truncation_diagnostic(&request.artifact_url));
    }
    (body, diagnostics)
}

/// Linked work items are found in the first part of a description; a very
/// long one is cut at a character boundary rather than rejected.
pub(super) fn bounded_description(mut description: String) -> String {
    const MAX_DESCRIPTION_BYTES: usize = 64 * 1024;
    if description.len() > MAX_DESCRIPTION_BYTES {
        let mut end = MAX_DESCRIPTION_BYTES;
        while !description.is_char_boundary(end) {
            end -= 1;
        }
        description.truncate(end);
    }
    description
}
pub(super) fn comments_heading(shown: usize, total: usize, complete: bool) -> String {
    if shown == total && complete {
        format!("\n\n## Comments ({total})")
    } else {
        format!("\n\n## Comments ({shown} of {total})")
    }
}

pub(super) fn comments_that_fit(remaining: usize, comments: &[String], complete: bool) -> usize {
    for shown in (0..=comments.len()).rev() {
        let heading_len = comments_heading(shown, comments.len(), complete).len();
        let comments_len = comments[..shown]
            .iter()
            .fold(0usize, |total, comment| total.saturating_add(comment.len()));
        if heading_len.saturating_add(comments_len) <= remaining {
            return shown;
        }
    }
    0
}

pub(super) fn issue_fields(issue: &IssueFacts, comment_count: usize) -> Vec<FrontmatterField> {
    let fields = vec![
        string_field("item_type", "Issue"),
        string_field("status", &status_value(&issue.state)),
        string_field("author", &issue.author),
        string_field("created", &rfc3339_seconds(&issue.created_at)),
        string_field("updated", &rfc3339_seconds(&issue.updated_at)),
        number_field("comment_count", comment_count),
        FrontmatterField {
            key: "assignee".into(),
            value: if issue.assignees.is_empty() {
                FrontmatterValue::Null
            } else {
                FrontmatterValue::String(issue.assignees.join(", "))
            },
        },
    ];
    fields
}

pub(super) fn review_fields(review: &ReviewFacts, comment_count: usize) -> Vec<FrontmatterField> {
    vec![
        string_field("item_type", "Merge request"),
        string_field("status", &status_value(&review.state)),
        string_field("author", &review.author),
        string_field("created", &rfc3339_seconds(&review.created_at)),
        string_field("updated", &rfc3339_seconds(&review.updated_at)),
        number_field("comment_count", comment_count),
        FrontmatterField {
            key: "assignee".into(),
            value: if review.assignees.is_empty() {
                FrontmatterValue::Null
            } else {
                FrontmatterValue::String(review.assignees.join(", "))
            },
        },
    ]
}

pub(super) fn summary_line(
    item_type: &str,
    state: &str,
    author: &str,
    assignees: &[String],
) -> String {
    let mut segments = vec![
        format!("**{item_type}**"),
        format!(
            "**{}**",
            status_value(state)
                .chars()
                .enumerate()
                .map(|(index, character)| if index == 0 {
                    character.to_ascii_uppercase()
                } else {
                    character
                })
                .collect::<String>()
        ),
        format!("Reporter {author}"),
    ];
    segments.push(if assignees.is_empty() {
        "Unassigned".into()
    } else {
        format!("Assignee {}", assignees.join(", "))
    });
    segments.join(" · ")
}

pub(super) fn status_value(state: &str) -> String {
    match state.to_ascii_lowercase().as_str() {
        "opened" | "open" => "open".into(),
        "closed" => "closed".into(),
        "merged" => "merged".into(),
        "draft" => "draft".into(),
        other => other.into(),
    }
}

pub(super) fn string_field(key: &str, value: &str) -> FrontmatterField {
    FrontmatterField {
        key: key.into(),
        value: FrontmatterValue::String(value.into()),
    }
}

pub(super) fn number_field(key: &str, value: usize) -> FrontmatterField {
    FrontmatterField {
        key: key.into(),
        value: FrontmatterValue::Number(value as i64),
    }
}

pub(super) fn rfc3339_seconds(timestamp: &str) -> String {
    let mut value = timestamp.to_owned();
    if let Some(dot) = value.find('.') {
        let suffix = value[dot..]
            .find(|character| matches!(character, '+' | '-' | 'Z'))
            .map(|offset| dot + offset)
            .unwrap_or(value.len());
        value.replace_range(dot..suffix, "");
    }
    value
}

pub(super) fn local_timestamp(timestamp: &str) -> String {
    let timestamp = rfc3339_seconds(timestamp);
    timestamp.get(..16).unwrap_or(&timestamp).replace('T', " ")
}

pub(super) fn edited_timestamp(created_at: &str, updated_at: &str) -> Option<String> {
    (rfc3339_seconds(created_at) != rfc3339_seconds(updated_at))
        .then(|| format!(" · edited {}", local_timestamp(updated_at)))
}

pub(super) fn review_location(position: &str) -> Option<String> {
    position
        .strip_prefix("review on ")
        .map(|location| format!(" · review on {location}"))
}

pub(super) fn timestamp_order(left: &str, right: &str) -> std::cmp::Ordering {
    match (timestamp_epoch(left), timestamp_epoch(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

pub(super) fn timestamp_epoch(timestamp: &str) -> Option<i64> {
    fn number(value: &str, range: std::ops::Range<usize>) -> Option<i64> {
        value.get(range)?.parse().ok()
    }

    let year = number(timestamp, 0..4)?;
    let month = number(timestamp, 5..7)?;
    let day = number(timestamp, 8..10)?;
    let hour = number(timestamp, 11..13)?;
    let minute = number(timestamp, 14..16)?;
    let second = number(timestamp, 17..19)?;
    let bytes = timestamp.as_bytes();
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let (offset_sign, offset_hours, offset_minutes) = if bytes.last() == Some(&b'Z') {
        (1, 0, 0)
    } else {
        let offset_start = timestamp.len().checked_sub(6)?;
        let sign = match bytes.get(offset_start)? {
            b'+' => 1,
            b'-' => -1,
            _ => return None,
        };
        let hours = number(timestamp, offset_start + 1..offset_start + 3)?;
        let minutes = number(timestamp, offset_start + 4..offset_start + 6)?;
        if bytes.get(offset_start + 3) != Some(&b':') || hours > 23 || minutes > 59 {
            return None;
        }
        (sign, hours, minutes)
    };
    let adjusted_year = year - if month <= 2 { 1 } else { 0 };
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era;
    let local_seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    let offset_seconds = offset_sign * (offset_hours * 3_600 + offset_minutes * 60);
    Some(local_seconds - offset_seconds)
}

pub(super) fn demote_headings(markdown: &str, minimum_level: usize) -> String {
    let mut output = String::with_capacity(markdown.len());
    let mut fence: Option<(u8, usize)> = None;
    for line in markdown.split_inclusive('\n') {
        let text = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = text.trim_start_matches(' ');
        let indent = text.len() - trimmed.len();
        if indent <= 3 {
            let bytes = trimmed.as_bytes();
            if let Some(marker @ (b'`' | b'~')) = bytes.first().copied() {
                let run = bytes.iter().take_while(|byte| **byte == marker).count();
                if run >= 3 {
                    let suffix = trimmed[run..].trim();
                    match fence {
                        Some((open_marker, open_len))
                            if marker == open_marker && run >= open_len && suffix.is_empty() =>
                        {
                            fence = None;
                        }
                        None => fence = Some((marker, run)),
                        _ => {}
                    }
                }
            }
        }
        if fence.is_none() {
            if let Some((prefix_end, level)) = atx_heading(text) {
                let leading = text.len() - text.trim_start().len();
                output.push_str(&text[..leading]);
                output.push_str(&"#".repeat(level.max(minimum_level)));
                output.push_str(&text[prefix_end..]);
            } else {
                output.push_str(text);
            }
        } else {
            output.push_str(text);
        }
        if line.ends_with('\n') {
            output.push('\n');
        }
    }
    output
}

pub(super) fn atx_heading(line: &str) -> Option<(usize, usize)> {
    let leading = line.len() - line.trim_start().len();
    if leading > 3 {
        return None;
    }
    let bytes = line.as_bytes();
    let level = bytes[leading..]
        .iter()
        .take_while(|byte| **byte == b'#')
        .count();
    if !(1..=6).contains(&level)
        || bytes
            .get(leading + level)
            .is_some_and(|byte| *byte != b' ' && *byte != b'\t')
    {
        return None;
    }
    Some((leading + level, level))
}
