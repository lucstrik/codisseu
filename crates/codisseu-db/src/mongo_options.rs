//! Typed, non-secret Mongo settings. No URI or password is persisted here.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MongoAuthentication {
    #[default]
    Automatic,
    ScramSha1,
    ScramSha256,
}
impl MongoAuthentication {
    pub const ALL: [Self; 3] = [Self::Automatic, Self::ScramSha1, Self::ScramSha256];
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automático",
            Self::ScramSha1 => "SCRAM-SHA-1",
            Self::ScramSha256 => "SCRAM-SHA-256",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MongoReadPreference {
    #[default]
    Primary,
    PrimaryPreferred,
    Secondary,
    SecondaryPreferred,
    Nearest,
}
impl MongoReadPreference {
    pub const ALL: [Self; 5] = [
        Self::Primary,
        Self::PrimaryPreferred,
        Self::Secondary,
        Self::SecondaryPreferred,
        Self::Nearest,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::PrimaryPreferred => "primaryPreferred",
            Self::Secondary => "secondary",
            Self::SecondaryPreferred => "secondaryPreferred",
            Self::Nearest => "nearest",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MongoOptions {
    pub replica_set: String,
    pub authentication: MongoAuthentication,
    pub read_preference: MongoReadPreference,
    pub direct_connection: bool,
}
impl Default for MongoOptions {
    fn default() -> Self {
        Self {
            replica_set: String::new(),
            authentication: MongoAuthentication::Automatic,
            read_preference: MongoReadPreference::Primary,
            direct_connection: true,
        }
    }
}
impl MongoOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self.replica_set.len() > 128
            || !self
                .replica_set
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            return Err("Replica set Mongo inválido: use até128 letras, números, ponto, hífen ou sublinhado".into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::ConnectionConfig;
    #[test]
    fn legacy_profiles_keep_direct_primary_auto_and_new_options_roundtrip() {
        let config = ConnectionConfig::default();
        let mut old = serde_json::to_value(&config).unwrap();
        old.as_object_mut().unwrap().remove("mongo");
        assert_eq!(
            serde_json::from_value::<ConnectionConfig>(old).unwrap(),
            config
        );
        let opts = MongoOptions {
            replica_set: "fixture-rs".into(),
            authentication: MongoAuthentication::ScramSha1,
            read_preference: MongoReadPreference::SecondaryPreferred,
            direct_connection: false,
        };
        assert_eq!(
            serde_json::from_str::<MongoOptions>(&serde_json::to_string(&opts).unwrap()).unwrap(),
            opts
        );
        assert_eq!(
            serde_json::from_str::<MongoOptions>("{}").unwrap(),
            MongoOptions::default()
        );
        for name in [
            "mongodb://user:password@host",
            "a\nb",
            "x/y",
            &"a".repeat(129),
        ] {
            assert!(
                MongoOptions {
                    replica_set: name.into(),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(serde_json::from_str::<MongoOptions>(r#"{"password":"not-allowed"}"#).is_err());
    }
}
