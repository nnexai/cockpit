use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::{self, Visitor}};

/// Fresh native pane evidence, distinct from the caller's main/subagent role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeAgentKind {
    Omp,
    Other(String),
}

impl NativeAgentKind {
    pub fn is_omp(kind: &str) -> bool {
        matches!(kind, "omp")
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Omp => "omp",
            Self::Other(kind) => kind,
        }
    }
}

impl From<String> for NativeAgentKind {
    fn from(kind: String) -> Self {
        if Self::is_omp(&kind) {
            Self::Omp
        } else {
            Self::Other(kind)
        }
    }
}

impl From<&str> for NativeAgentKind {
    fn from(kind: &str) -> Self {
        if Self::is_omp(kind) {
            Self::Omp
        } else {
            Self::Other(kind.to_owned())
        }
    }
}

impl Serialize for NativeAgentKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NativeAgentKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct KindVisitor;

        impl<'de> Visitor<'de> for KindVisitor {
            type Value = NativeAgentKind;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a native agent kind string")
            }

            fn visit_str<E>(self, kind: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(kind.into())
            }

            fn visit_string<E>(self, kind: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(kind.into())
            }
        }

        deserializer.deserialize_string(KindVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::NativeAgentKind;

    #[test]
    fn only_exact_omp_is_native_omp() {
        assert_eq!(NativeAgentKind::from("omp"), NativeAgentKind::Omp);
        for kind in ["shell", "OMP", "Omp", "omp ", "", "native-λ\0kind"] {
            assert_eq!(NativeAgentKind::from(kind), NativeAgentKind::Other(kind.into()));
            assert!(!NativeAgentKind::is_omp(kind));
        }
        assert!(NativeAgentKind::is_omp("omp"));
    }

    #[test]
    fn serde_preserves_exact_string_wire_values() {
        for value in ["omp", "shell", "OMP", "Omp", "omp ", "", "native-λ\0kind"] {
            let kind = NativeAgentKind::from(value);
            let encoded = serde_json::to_value(&kind).unwrap();
            assert_eq!(encoded, serde_json::Value::String(value.into()));
            assert_eq!(serde_json::from_value::<NativeAgentKind>(encoded).unwrap(), kind);
        }
    }

    #[test]
    fn serde_rejects_non_string_native_kinds() {
        for value in [
            serde_json::json!(null),
            serde_json::json!(true),
            serde_json::json!(1),
            serde_json::json!(["omp"]),
            serde_json::json!({"kind": "omp"}),
        ] {
            assert!(serde_json::from_value::<NativeAgentKind>(value).is_err());
        }
    }
}
