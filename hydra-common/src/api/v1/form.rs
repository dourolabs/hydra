use super::issues::IssueStatus;
use crate::actor_ref::ActorId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A form attached to an issue for human interaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Form {
    /// Human-readable prompt explaining what the user needs to do (markdown).
    pub prompt: String,

    /// Ordered list of form fields rendered above the action buttons.
    /// May be empty for simple action-only forms.
    #[serde(default)]
    pub fields: Vec<Field>,

    /// Ordered list of actions the user can take (rendered as buttons).
    pub actions: Vec<Action>,
}

/// A single form field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Field {
    /// Unique key within the form. Used as the key in the response map.
    pub key: String,

    /// Human-readable label displayed above the input.
    pub label: String,

    /// Optional help text displayed below the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The input type and its configuration.
    pub input: Input,

    /// Default value. Type must match the input type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

/// Input type determines the rendered widget and validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type")]
pub enum Input {
    /// Single-line text input. Produces a string.
    #[serde(rename = "text")]
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_length: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<usize>,
        /// Regex pattern the value must match.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<String>,
    },

    /// Multi-line text area. Produces a string.
    #[serde(rename = "textarea")]
    Textarea {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_length: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<usize>,
        /// Visible rows (rendering hint). Defaults to 4.
        #[serde(default = "default_rows")]
        rows: usize,
    },

    /// Select from a list of options. Produces a string (the selected value).
    #[serde(rename = "select")]
    Select {
        options: Vec<SelectOption>,
        /// Render as radio buttons instead of a dropdown.
        #[serde(default)]
        radio: bool,
    },

    /// Boolean toggle. Produces a bool.
    #[serde(rename = "checkbox")]
    Checkbox,

    /// Numeric input. Produces a number.
    #[serde(rename = "number")]
    Number {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<f64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

/// An action button on the form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Action {
    /// Unique identifier within the form.
    pub id: String,

    /// Button label.
    pub label: String,

    /// Visual style: "primary", "danger", "default".
    #[serde(default = "default_style")]
    pub style: String,

    /// Which field keys are required for this action.
    #[serde(default)]
    pub requires: Vec<String>,

    /// What happens when the user clicks this action.
    pub effect: Effect,
}

/// What happens when an action is taken.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type")]
pub enum Effect {
    /// Update the issue's status.
    #[serde(rename = "update_issue")]
    UpdateIssue { status: IssueStatus },

    /// No automated effect — just record the action in the activity log.
    #[serde(rename = "record_only")]
    RecordOnly,
}

/// The user's response to a form submission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct FormResponse {
    /// Which action was taken.
    pub action_id: String,

    /// Who submitted the response.
    pub actor: ActorId,

    /// Field values, keyed by field key. Typed JSON values.
    #[serde(default)]
    pub values: HashMap<String, Value>,

    /// When the form was submitted.
    pub submitted_at: DateTime<Utc>,
}

impl Form {
    /// Validates that all field keys are unique. Returns an error message if not.
    pub fn validate_field_keys(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for field in &self.fields {
            if !seen.insert(&field.key) {
                return Err(format!("duplicate field key: '{}'", field.key));
            }
        }
        Ok(())
    }
}

fn default_rows() -> usize {
    4
}

fn default_style() -> String {
    "default".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn form_round_trip_no_fields() {
        let form = Form {
            prompt: "Pick one".to_string(),
            fields: vec![],
            actions: vec![
                Action {
                    id: "yes".to_string(),
                    label: "Yes".to_string(),
                    style: "primary".to_string(),
                    requires: vec![],
                    effect: Effect::UpdateIssue {
                        status: IssueStatus::Closed,
                    },
                },
                Action {
                    id: "no".to_string(),
                    label: "No".to_string(),
                    style: "danger".to_string(),
                    requires: vec![],
                    effect: Effect::RecordOnly,
                },
            ],
        };

        let json = serde_json::to_value(&form).unwrap();
        let round_trip: Form = serde_json::from_value(json).unwrap();
        assert_eq!(form, round_trip);
    }

    #[test]
    fn form_round_trip_with_fields() {
        let form = Form {
            prompt: "Answer these questions".to_string(),
            fields: vec![
                Field {
                    key: "name".to_string(),
                    label: "Name".to_string(),
                    description: Some("Your full name".to_string()),
                    input: Input::Text {
                        placeholder: Some("John Doe".to_string()),
                        min_length: Some(1),
                        max_length: Some(100),
                        pattern: None,
                    },
                    default: None,
                },
                Field {
                    key: "feedback".to_string(),
                    label: "Feedback".to_string(),
                    description: None,
                    input: Input::Textarea {
                        placeholder: None,
                        min_length: None,
                        max_length: None,
                        rows: 6,
                    },
                    default: None,
                },
                Field {
                    key: "env".to_string(),
                    label: "Environment".to_string(),
                    description: None,
                    input: Input::Select {
                        options: vec![
                            SelectOption {
                                value: "staging".to_string(),
                                label: "Staging".to_string(),
                            },
                            SelectOption {
                                value: "prod".to_string(),
                                label: "Production".to_string(),
                            },
                        ],
                        radio: false,
                    },
                    default: None,
                },
                Field {
                    key: "enable_ci".to_string(),
                    label: "Enable CI".to_string(),
                    description: None,
                    input: Input::Checkbox,
                    default: Some(json!(false)),
                },
                Field {
                    key: "score".to_string(),
                    label: "Score".to_string(),
                    description: None,
                    input: Input::Number {
                        min: Some(1.0),
                        max: Some(5.0),
                        step: Some(1.0),
                    },
                    default: None,
                },
            ],
            actions: vec![Action {
                id: "submit".to_string(),
                label: "Submit".to_string(),
                style: "primary".to_string(),
                requires: vec!["name".to_string(), "env".to_string()],
                effect: Effect::UpdateIssue {
                    status: IssueStatus::Closed,
                },
            }],
        };

        let json = serde_json::to_value(&form).unwrap();
        let round_trip: Form = serde_json::from_value(json).unwrap();
        assert_eq!(form, round_trip);
    }

    #[test]
    fn form_response_round_trip() {
        use crate::users::Username;

        let response = FormResponse {
            action_id: "submit".to_string(),
            actor: ActorId::Username(Username::from("alice")),
            values: {
                let mut m = HashMap::new();
                m.insert("score".to_string(), json!(4));
                m.insert("feedback".to_string(), json!("Looks good"));
                m
            },
            submitted_at: Utc::now(),
        };

        let json = serde_json::to_value(&response).unwrap();
        let round_trip: FormResponse = serde_json::from_value(json).unwrap();
        assert_eq!(response, round_trip);
    }

    #[test]
    fn validate_field_keys_rejects_duplicates() {
        let form = Form {
            prompt: "test".to_string(),
            fields: vec![
                Field {
                    key: "name".to_string(),
                    label: "Name".to_string(),
                    description: None,
                    input: Input::Text {
                        placeholder: None,
                        min_length: None,
                        max_length: None,
                        pattern: None,
                    },
                    default: None,
                },
                Field {
                    key: "name".to_string(),
                    label: "Name Again".to_string(),
                    description: None,
                    input: Input::Text {
                        placeholder: None,
                        min_length: None,
                        max_length: None,
                        pattern: None,
                    },
                    default: None,
                },
            ],
            actions: vec![],
        };

        assert!(form.validate_field_keys().is_err());
    }

    #[test]
    fn validate_field_keys_accepts_unique() {
        let form = Form {
            prompt: "test".to_string(),
            fields: vec![
                Field {
                    key: "name".to_string(),
                    label: "Name".to_string(),
                    description: None,
                    input: Input::Text {
                        placeholder: None,
                        min_length: None,
                        max_length: None,
                        pattern: None,
                    },
                    default: None,
                },
                Field {
                    key: "email".to_string(),
                    label: "Email".to_string(),
                    description: None,
                    input: Input::Text {
                        placeholder: None,
                        min_length: None,
                        max_length: None,
                        pattern: None,
                    },
                    default: None,
                },
            ],
            actions: vec![],
        };

        assert!(form.validate_field_keys().is_ok());
    }

    #[test]
    fn input_text_deserializes_from_json() {
        let json = json!({"type": "text", "placeholder": "Enter name"});
        let input: Input = serde_json::from_value(json).unwrap();
        assert!(matches!(input, Input::Text { placeholder: Some(p), .. } if p == "Enter name"));
    }

    #[test]
    fn effect_update_issue_round_trip() {
        let effect = Effect::UpdateIssue {
            status: IssueStatus::Closed,
        };
        let json = serde_json::to_value(&effect).unwrap();
        assert_eq!(json, json!({"type": "update_issue", "status": "closed"}));
        let round_trip: Effect = serde_json::from_value(json).unwrap();
        assert_eq!(effect, round_trip);
    }

    #[test]
    fn textarea_default_rows() {
        let json = json!({"type": "textarea"});
        let input: Input = serde_json::from_value(json).unwrap();
        if let Input::Textarea { rows, .. } = input {
            assert_eq!(rows, 4);
        } else {
            panic!("expected Textarea");
        }
    }
}
