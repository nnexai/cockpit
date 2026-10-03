use std::collections::HashSet;

use cockpit_protocol::widget::{
    WIDGET_MAX_CHOICES_BYTES, WIDGET_MAX_SELECTION_BYTES, WidgetChoicesSpec,
};
use serde::Serialize;

use crate::InspectionError;

use super::text;

pub fn parse(input: &str) -> Result<WidgetChoicesSpec, InspectionError> {
    if input.len() > WIDGET_MAX_CHOICES_BYTES {
        return Err(InspectionError::new(
            "widget_too_large",
            "choices exceed 64 KiB",
        ));
    }
    let mut spec: WidgetChoicesSpec = serde_json::from_str(input)
        .map_err(|_| InspectionError::new("widget_usage", "invalid choices JSON"))?;
    if !(1..=20).contains(&spec.choices.len()) {
        return Err(InspectionError::new(
            "widget_usage",
            "choices must contain 1 to 20 entries",
        ));
    }
    let mut ids = HashSet::with_capacity(spec.choices.len());
    for choice in &mut spec.choices {
        if !text::valid_id(&choice.id) {
            return Err(InspectionError::new(
                "widget_usage",
                "choice ids must match [a-z0-9][a-z0-9_-]{0,47}",
            ));
        }
        if !ids.insert(choice.id.as_str()) {
            return Err(InspectionError::new(
                "widget_usage",
                "choice ids must be unique",
            ));
        }
        choice.label = bounded_text(&choice.label, 80, "choice label")?;
        if choice.label.is_empty() {
            return Err(InspectionError::new(
                "widget_usage",
                "choice labels must not be empty",
            ));
        }
        if let Some(detail) = &mut choice.detail {
            *detail = bounded_text(detail, 200, "choice detail")?;
        }
    }
    if let Some(prompt) = &mut spec.prompt {
        *prompt = bounded_text(prompt, 200, "choices prompt")?;
    }
    Ok(spec)
}

fn bounded_text(input: &str, limit: usize, field: &str) -> Result<String, InspectionError> {
    let normalized = text::sanitize(input, limit + 1);
    if normalized.chars().count() > limit {
        return Err(InspectionError::new(
            "widget_usage",
            format!("{field} exceeds {limit} characters"),
        ));
    }
    Ok(normalized)
}

pub fn value(spec: &WidgetChoicesSpec, choice_id: &str) -> Result<String, InspectionError> {
    let choice = spec
        .choices
        .iter()
        .find(|choice| choice.id == choice_id)
        .ok_or_else(|| InspectionError::new("widget_usage", "unknown choice id"))?;
    // Field order is part of the canonical choice value, independent of JSON map ordering.
    #[derive(Serialize)]
    struct Selection<'a> {
        id: &'a str,
        label: &'a str,
    }
    let json = serde_json::to_string(&Selection {
        id: &choice.id,
        label: &choice.label,
    })
    .map_err(|_| InspectionError::new("widget_usage", "choice value could not be serialized"))?;
    if json.len() > WIDGET_MAX_SELECTION_BYTES {
        return Err(InspectionError::new(
            "widget_too_large",
            "selection exceeds 16 KiB",
        ));
    }
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_sanitized_text_and_canonical_explicit_selection() {
        let spec = parse(r#"{"prompt":"  Pick\nnow\u202e ","choices":[{"id":"a","label":" Quote \"yes\"\u0000 ","detail":"line\n two"},{"id":"b","label":"Other"}]}"#).unwrap();
        assert_eq!(spec.prompt.as_deref(), Some("Pick now"));
        assert_eq!(spec.choices[0].label, "Quote \"yes\"");
        assert_eq!(spec.choices[0].detail.as_deref(), Some("line two"));
        assert_eq!(
            value(&spec, "a").unwrap(),
            r#"{"id":"a","label":"Quote \"yes\""}"#
        );
        assert_eq!(value(&spec, "b").unwrap(), r#"{"id":"b","label":"Other"}"#);
        assert_eq!(value(&spec, "missing").unwrap_err().code, "widget_usage");
    }

    #[test]
    fn rejects_schema_errors_duplicates_and_invisible_labels() {
        for input in [
            r#"{"choices":[]}"#,
            r#"{"choices":[{"id":"A","label":"a"}]}"#,
            r#"{"choices":[{"id":"a","label":"a"},{"id":"a","label":"b"}]}"#,
            r#"{"choices":[{"id":"a","label":" \u202e\u0000 "}]}"#,
            r#"{"choices":[{"id":"a","label":"a","value":"injected"}]}"#,
            r#"{"choices":[{"id":"a","label":"a"}],"multiple":true}"#,
            r#"{"choices":[{"id":"a","label":3}]}"#,
            r#"{"choices":[{"id":"a"}]}"#,
            r#"{"choices":[{"id":"a","label":"a"}]} trailing"#,
        ] {
            assert_eq!(parse(input).unwrap_err().code, "widget_usage", "{input}");
        }
    }

    #[test]
    fn bounds_input_entries_and_visible_text() {
        assert_eq!(
            parse(&" ".repeat(WIDGET_MAX_CHOICES_BYTES + 1))
                .unwrap_err()
                .code,
            "widget_too_large"
        );
        let choices: Vec<_> = (0..21)
            .map(|n| serde_json::json!({"id": format!("c{n}"), "label": "choice"}))
            .collect();
        assert_eq!(
            parse(&serde_json::json!({"choices": choices}).to_string())
                .unwrap_err()
                .code,
            "widget_usage"
        );
        let input = serde_json::json!({"prompt": "p".repeat(200), "choices": [{"id": "a", "label": "界".repeat(80), "detail": "d".repeat(200)}]}).to_string();
        let spec = parse(&input).unwrap();
        assert_eq!(spec.prompt, Some("p".repeat(200)));
        assert_eq!(spec.choices[0].label, "界".repeat(80));
        assert_eq!(spec.choices[0].detail, Some("d".repeat(200)));
        for input in [
            serde_json::json!({"choices": [{"id": "a", "label": "界".repeat(81)}]}),
            serde_json::json!({"prompt": "p".repeat(201), "choices": [{"id": "a", "label": "a"}]}),
            serde_json::json!({"choices": [{"id": "a", "label": "a", "detail": "d".repeat(201)}]}),
        ] {
            assert_eq!(parse(&input.to_string()).unwrap_err().code, "widget_usage");
        }
        let normalized = serde_json::json!({"choices": [{"id": "a", "label": format!("  {} \u{202e}\u{0000} ", "界".repeat(80))}]}).to_string();
        assert_eq!(
            parse(&normalized).unwrap().choices[0].label,
            "界".repeat(80)
        );
    }
}
