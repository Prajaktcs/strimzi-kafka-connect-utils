//! Server-side security for read-only sink lag clients; never connector overrides.
use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KafkaSecurityProtocol {
    #[default]
    Plaintext,
    Ssl,
    SaslPlaintext,
    SaslSsl,
}

impl KafkaSecurityProtocol {
    #[cfg(feature = "kafka")]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Plaintext => "PLAINTEXT",
            Self::Ssl => "SSL",
            Self::SaslPlaintext => "SASL_PLAINTEXT",
            Self::SaslSsl => "SASL_SSL",
        }
    }

    fn tls(self) -> bool {
        matches!(self, Self::Ssl | Self::SaslSsl)
    }

    fn sasl(self) -> bool {
        matches!(self, Self::SaslPlaintext | Self::SaslSsl)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum KafkaSaslMechanism {
    #[serde(rename = "PLAIN")]
    Plain,
    #[serde(rename = "SCRAM-SHA-256")]
    ScramSha256,
    #[serde(rename = "SCRAM-SHA-512")]
    ScramSha512,
}

impl KafkaSaslMechanism {
    #[cfg(feature = "kafka")]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "PLAIN",
            Self::ScramSha256 => "SCRAM-SHA-256",
            Self::ScramSha512 => "SCRAM-SHA-512",
        }
    }
}

/// Credentials and paths deliberately have no `Debug` representation. No arbitrary
/// librdkafka settings are accepted, including verification or group-safety knobs.
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SinkLagSecurity {
    pub protocol: KafkaSecurityProtocol,
    pub ca_file: Option<PathBuf>,
    pub certificate_file: Option<PathBuf>,
    pub key_file: Option<PathBuf>,
    pub sasl_mechanism: Option<KafkaSaslMechanism>,
    pub sasl_username: Option<String>,
    pub sasl_password: Option<String>,
}

impl fmt::Debug for SinkLagSecurity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SinkLagSecurity")
            .field("protocol", &self.protocol)
            .finish_non_exhaustive()
    }
}

impl SinkLagSecurity {
    /// Errors identify the invalid setting, never its value or a foreign error.
    /// File contents (PEM parsing/key matching) are checked by librdkafka at creation.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.protocol.tls()
            && (self.ca_file.is_some()
                || self.certificate_file.is_some()
                || self.key_file.is_some())
        {
            return Err("sink lag TLS files require SSL or SASL_SSL");
        }
        if self.certificate_file.is_some() != self.key_file.is_some() {
            return Err("sink lag TLS client certificate and key must be configured together");
        }
        if self.protocol.sasl() {
            if self.sasl_mechanism.is_none()
                || !self.sasl_username.as_deref().is_some_and(valid_credential)
                || !self.sasl_password.as_deref().is_some_and(valid_credential)
            {
                return Err("sink lag SASL requires a mechanism, username and password");
            }
        } else if self.sasl_mechanism.is_some()
            || self.sasl_username.is_some()
            || self.sasl_password.is_some()
        {
            return Err("sink lag SASL settings require SASL_PLAINTEXT or SASL_SSL");
        }
        for path in [&self.ca_file, &self.certificate_file, &self.key_file]
            .into_iter()
            .flatten()
        {
            if path.to_str().map_or(true, |value| {
                value.is_empty() || value.contains('\0') || value.contains("${")
            }) {
                return Err("sink lag TLS file path is invalid or unresolved");
            }
            if !std::fs::metadata(path)
                .map_err(|_| "cannot inspect sink lag TLS file")?
                .is_file()
            {
                return Err("sink lag TLS path must reference a regular file");
            }
            File::open(path).map_err(|_| "cannot read sink lag TLS file")?;
        }
        Ok(())
    }

    pub(super) fn resolve_paths(&mut self, directory: &Path) {
        for path in [
            &mut self.ca_file,
            &mut self.certificate_file,
            &mut self.key_file,
        ]
        .into_iter()
        .flatten()
        {
            if path.is_relative() {
                *path = directory.join(&*path);
            }
        }
    }
}

fn valid_credential(value: &str) -> bool {
    !value.is_empty() && !value.contains("${") && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sasl(protocol: KafkaSecurityProtocol) -> SinkLagSecurity {
        SinkLagSecurity {
            protocol,
            sasl_mechanism: Some(KafkaSaslMechanism::ScramSha512),
            sasl_username: Some("private-user".to_owned()),
            sasl_password: Some("private-password".to_owned()),
            ..SinkLagSecurity::default()
        }
    }

    #[test]
    fn protocols_require_exact_security_combinations() {
        assert!(SinkLagSecurity::default().validate().is_ok());
        for protocol in [
            KafkaSecurityProtocol::SaslPlaintext,
            KafkaSecurityProtocol::SaslSsl,
        ] {
            let mut config = sasl(protocol);
            assert!(config.validate().is_ok());
            config.sasl_password = None;
            assert!(config.validate().is_err());
            config.sasl_password = Some("${env:PASSWORD}".to_owned());
            assert!(config.validate().is_err());
            config.sasl_password = Some("secret\0invalid".to_owned());
            assert!(config.validate().is_err());
        }
        assert!(sasl(KafkaSecurityProtocol::Plaintext).validate().is_err());
        assert!(sasl(KafkaSecurityProtocol::Ssl).validate().is_err());
        let mut tls = SinkLagSecurity {
            protocol: KafkaSecurityProtocol::Ssl,
            ..SinkLagSecurity::default()
        };
        assert!(tls.validate().is_ok());
        tls.certificate_file = Some(PathBuf::from("private-certificate"));
        assert!(tls.validate().is_err());
        tls.protocol = KafkaSecurityProtocol::Plaintext;
        assert!(tls.validate().is_err());
    }

    #[test]
    fn file_validation_and_debug_never_expose_private_values() {
        let mut config = sasl(KafkaSecurityProtocol::SaslSsl);
        config.ca_file = Some(PathBuf::from("private-path-that-does-not-exist"));
        let output = format!("{config:?} {}", config.validate().unwrap_err());
        for secret in ["private-user", "private-password", "private-path"] {
            assert!(!output.contains(secret));
        }
        let directory = tempfile::tempdir().unwrap();
        config.ca_file = Some(directory.path().to_owned());
        assert_eq!(
            config.validate(),
            Err("sink lag TLS path must reference a regular file")
        );
        let file = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        config.ca_file = Some(file.path().to_owned());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn only_explicitly_supported_mechanisms_and_settings_deserialize() {
        for mechanism in ["PLAIN", "SCRAM-SHA-256", "SCRAM-SHA-512"] {
            let config: SinkLagSecurity = toml::from_str(&format!(
                "protocol='SASL_SSL'\nsasl_mechanism='{mechanism}'\nsasl_username='user'\nsasl_password='password'"
            )).unwrap();
            assert!(config.validate().is_ok());
        }
        for text in [
            "protocol='SSL'\nenable_ssl_certificate_verification=false",
            "protocol='SASL_SSL'\nsasl_mechanism='GSSAPI'",
            "protocol='SASL_SSL'\nsasl_mechanism='OAUTHBEARER'",
            "protocol='SASL_SSL'\nsasl_mechanism='secret-mechanism'",
        ] {
            assert!(toml::from_str::<SinkLagSecurity>(text).is_err());
        }
    }
}
