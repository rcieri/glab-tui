use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    /// Hex color without the leading `#`, if the API provided one.
    #[serde(default)]
    pub color: Option<String>,
}
