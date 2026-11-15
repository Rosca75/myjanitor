//! Generic parameters and their schema, so the step panel ("How to shrink them")
//! and detector settings render from data instead of per-feature UI code.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One parameter value. JSON: `true`, `24`, `"opus"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

/// Parameter values keyed by `SettingSpec::key`. Missing keys take the spec default.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ParamValues(pub BTreeMap<String, ParamValue>);

impl ParamValues {
    /// Empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder-style insert.
    pub fn with(mut self, key: &str, v: ParamValue) -> Self {
        self.0.insert(key.to_string(), v);
        self
    }

    /// Set a value.
    pub fn set(&mut self, key: &str, v: ParamValue) {
        self.0.insert(key.to_string(), v);
    }

    /// Boolean or `default`.
    pub fn bool(&self, key: &str, default: bool) -> bool {
        match self.0.get(key) {
            Some(ParamValue::Bool(b)) => *b,
            _ => default,
        }
    }

    /// Number or `default`.
    pub fn number(&self, key: &str, default: f64) -> f64 {
        match self.0.get(key) {
            Some(ParamValue::Number(n)) if n.is_finite() => *n,
            _ => default,
        }
    }

    /// Text or `default`.
    pub fn text<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        match self.0.get(key) {
            Some(ParamValue::Text(s)) => s,
            _ => default,
        }
    }

    /// Values of `other` override `self`.
    pub fn merged(&self, other: &ParamValues) -> ParamValues {
        let mut out = self.clone();
        for (k, v) in &other.0 {
            out.0.insert(k.clone(), v.clone());
        }
        out
    }
}

/// One choice of a [`SettingKind::Choice`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

/// Control type and default of a setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingKind {
    Toggle {
        default: bool,
    },
    Number {
        default: f64,
        min: f64,
        max: f64,
        step: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    Choice {
        default: String,
        options: Vec<ChoiceOption>,
    },
    Text {
        default: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    },
    /// A folder chosen with the in-app folder browser.
    Folder {
        default: String,
    },
}

/// A setting of a detector or optimiser, as rendered in the step panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingSpec {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    pub kind: SettingKind,
    /// Shown inside the collapsed "Advanced" section.
    #[serde(default)]
    pub advanced: bool,
    /// Turning it on makes the step lossy.
    #[serde(default)]
    pub lossy: bool,
    /// When set the control is shown disabled with this one-line reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

impl SettingSpec {
    /// The default value as a [`ParamValue`].
    pub fn default_value(&self) -> ParamValue {
        match &self.kind {
            SettingKind::Toggle { default } => ParamValue::Bool(*default),
            SettingKind::Number { default, .. } => ParamValue::Number(*default),
            SettingKind::Choice { default, .. }
            | SettingKind::Text { default, .. }
            | SettingKind::Folder { default } => ParamValue::Text(default.clone()),
        }
    }
}

/// Defaults of a whole schema.
pub fn defaults(specs: &[SettingSpec]) -> ParamValues {
    ParamValues(specs.iter().map(|s| (s.key.clone(), s.default_value())).collect())
}

/// A preset tile of the step panel (e.g. "Speech · Opus 24 kbps mono").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepPreset {
    pub id: String,
    pub label: String,
    pub description: String,
    /// Values this preset sets; other keys keep their current value.
    pub values: ParamValues,
    /// The preset produces lossy output.
    pub lossy: bool,
}
