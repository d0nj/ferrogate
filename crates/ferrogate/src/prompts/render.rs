use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemplateVariable {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl TemplateVariable {
    pub fn parse_list(raw: &str) -> Vec<Self> {
        parse_variables(raw).unwrap_or_default()
    }
}

pub fn parse_variables(raw: &str) -> Result<Vec<TemplateVariable>, serde_json::Error> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(trimmed)
}

pub fn encode_variables(variables: &[TemplateVariable]) -> String {
    serde_json::to_string(variables).unwrap_or_else(|_| "[]".to_string())
}

#[derive(Clone, Debug, Serialize)]
pub struct RenderedPrompt {
    pub system: String,
    pub user: String,
    pub missing_variables: Vec<String>,
}

pub fn render_template(
    declared: &[TemplateVariable],
    system_prompt: &str,
    user_template: &str,
    values: &Map<String, Value>,
) -> RenderedPrompt {
    let mut missing = Vec::new();
    let mut system = system_prompt.to_string();
    let mut user = user_template.to_string();

    for variable in declared {
        let (value, resolved) = match values.get(&variable.name) {
            Some(value) if !value.is_null() => (scalar(value), true),
            _ => match &variable.default {
                Some(default) => (default.clone(), true),
                None => (String::new(), false),
            },
        };
        if !resolved {
            missing.push(variable.name.clone());
        }

        let pattern = format!("{{{{{}}}}}", variable.name);
        let spaced = format!("{{{{ {} }}}}", variable.name);
        system = system.replace(&pattern, &value).replace(&spaced, &value);
        user = user.replace(&pattern, &value).replace(&spaced, &value);
    }

    RenderedPrompt {
        system,
        user,
        missing_variables: missing,
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn variable(name: &str, default: Option<&str>) -> TemplateVariable {
        TemplateVariable {
            name: name.to_string(),
            default: default.map(str::to_string),
        }
    }

    #[test]
    fn substitutes_spaced_and_bare_placeholders() {
        let declared = vec![variable("topic", None), variable("tone", None)];
        let values = json!({ "topic": "rust", "tone": "terse" })
            .as_object()
            .unwrap()
            .clone();

        let rendered = render_template(
            &declared,
            "You are an expert on {{topic}}.",
            "{{ topic }} explained {{tone}}",
            &values,
        );

        assert_eq!(rendered.system, "You are an expert on rust.");
        assert_eq!(rendered.user, "rust explained terse");
        assert!(rendered.missing_variables.is_empty());
    }

    #[test]
    fn missing_variables_render_empty_and_are_reported() {
        let declared = vec![variable("topic", None), variable("tone", Some("neutral"))];
        let rendered = render_template(&declared, "{{topic}}", "say {{tone}}", &Map::new());

        assert_eq!(rendered.system, "");
        assert_eq!(rendered.user, "say neutral");
        assert_eq!(rendered.missing_variables, vec!["topic".to_string()]);
    }

    #[test]
    fn non_string_values_render_their_json_form() {
        let declared = vec![variable("count", None)];
        let values = json!({ "count": 3 }).as_object().unwrap().clone();
        let rendered = render_template(&declared, "{{count}} items", "", &values);
        assert_eq!(rendered.system, "3 items");
    }
}
