//! Additional public roots, scoped to the provider's DNS namespace.
use crate::remote::{ConnectionConfig, Engine};
use base64::Engine as _;
use rustls_pki_types::{CertificateDer, pem::PemObject};
use std::{
    io::{Read, Write},
    sync::Arc,
};
pub(crate) const MAX_CA_BYTES: usize = 256 * 1024;

const AWS_RDS_ROOTS: &[u8] = include_bytes!("../certificates/aws-rds-roots.pem");

pub(crate) fn automatic_rds_roots(config: &ConnectionConfig) -> Option<&'static [u8]> {
    (config.tls
        && config.engine.is_sql()
        && config.ca_file.is_empty()
        && config.ca_pem.is_empty()
        && is_rds_host(&config.host))
    .then_some(AWS_RDS_ROOTS)
}

pub(crate) fn automatic_aws_roots(config: &ConnectionConfig) -> Option<&'static [u8]> {
    if config.engine.is_sql() {
        return automatic_rds_roots(config);
    }
    (config.tls
        && config.engine == Engine::Mongo
        && config.ca_file.is_empty()
        && config.ca_pem.is_empty()
        && provider_host(&config.host, ".docdb.amazonaws.com"))
    .then_some(AWS_RDS_ROOTS)
}

fn validate_pem(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > MAX_CA_BYTES {
        return Err("Arquivo CA deve ter entre1byte e256KiB".into());
    }
    if bytes
        .windows(b"PRIVATE KEY".len())
        .any(|s| s == b"PRIVATE KEY")
    {
        return Err("Importe somente certificados CA públicos, sem chave privada".into());
    }
    let mut count = 0;
    for certificate in CertificateDer::pem_slice_iter(bytes) {
        certificate.map_err(|_| "Certificado CA PEM inválido")?;
        count += 1;
    }
    if count == 0 {
        return Err("Arquivo CA não contém certificados PEM".into());
    }
    Ok(())
}
pub(crate) fn prepare_saved_certificate(config: &mut ConnectionConfig) -> Result<(), String> {
    if !config.ca_file.is_empty() {
        if !config.tls {
            return Err("Certificado CA exige TLS ativado".into());
        }
        let mut bytes = Vec::new();
        let file=std::fs::File::open(&config.ca_file).map_err(|_|"Não foi possível ler o arquivo CA; use automático ou selecione um certificado público existente")?;
        if !file
            .metadata()
            .map_err(|_| "Arquivo CA indisponível")?
            .is_file()
        {
            return Err("CA deve ser um arquivo PEM regular".into());
        }
        file.take((MAX_CA_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "Não foi possível ler o certificado CA")?;
        validate_pem(&bytes)?;
        config.ca_pem = String::from_utf8(bytes)
            .map_err(|_| "Certificado CA PEM deve ser texto UTF-8")?
            .into();
        config.ca_file.clear();
    } else if !config.ca_pem.is_empty() {
        validate_pem(config.ca_pem.as_bytes())?;
    }
    Ok(())
}
/// Connection workers only. Native trust contains public/system-authorized CAs,
/// never certificates downloaded from the target server. Built-in driver roots
/// remain the fallback when the operating system has no certificate store.
pub(crate) fn material(config: &ConnectionConfig) -> Result<Option<Arc<[u8]>>, String> {
    if !config.tls {
        return Ok(None);
    }
    if !config.ca_file.is_empty() || !config.ca_pem.is_empty() {
        let mut copy = config.clone();
        prepare_saved_certificate(&mut copy)?;
        return Ok(Some(Arc::from(copy.ca_pem.as_bytes())));
    }
    if let Some(bytes) = automatic_aws_roots(config) {
        return Ok(Some(Arc::from(bytes)));
    }
    let roots = rustls_native_certs::load_native_certs();
    let mut pem = Vec::new();
    for root in roots.certs.into_iter().take(1024) {
        if pem.len() + root.len() * 2 > 2 * 1024 * 1024 {
            break;
        }
        pem.extend_from_slice(b"-----BEGIN CERTIFICATE-----\n");
        let encoded = base64::engine::general_purpose::STANDARD.encode(root.as_ref());
        for line in encoded.as_bytes().chunks(64) {
            pem.extend_from_slice(line);
            pem.push(b'\n');
        }
        pem.extend_from_slice(b"-----END CERTIFICATE-----\n");
    }
    Ok((!pem.is_empty()).then(|| Arc::from(pem)))
}
pub(crate) fn temporary_pem(bytes: &[u8]) -> Result<tempfile::NamedTempFile, String> {
    let mut file = tempfile::Builder::new()
        .prefix("codisseu-ca-")
        .suffix(".pem")
        .tempfile()
        .map_err(|_| "Não foi possível preparar certificados TLS")?;
    file.write_all(bytes)
        .map_err(|_| "Não foi possível preparar certificados TLS")?;
    file.flush()
        .map_err(|_| "Não foi possível preparar certificados TLS")?;
    Ok(file)
}

fn is_rds_host(host: &str) -> bool {
    provider_host(host, ".rds.amazonaws.com")
}
fn provider_host(host: &str, suffix: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.len() > 253 {
        return false;
    }
    let host = host.to_ascii_lowercase();
    let Some(prefix) = host.strip_suffix(suffix) else {
        return false;
    };
    let labels: Vec<_> = prefix.split('.').collect();
    if labels.len() < 3
        || labels.iter().any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
    {
        return false;
    }
    let region: Vec<_> = labels.last().unwrap().split('-').collect();
    region.len() >= 3
        && region[0].len() == 2
        && region[..region.len() - 1]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_lowercase()))
        && !region.last().unwrap().is_empty()
        && region.last().unwrap().bytes().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::Engine;
    #[test]
    fn automatic_roots_require_exact_provider_dns_sql_and_verified_tls_without_override() {
        for host in [
            "db.abc.us-east-1.rds.amazonaws.com",
            "db.cluster-xyz.sa-east-1.rds.amazonaws.com",
            "DB.ABC.US-EAST-1.RDS.AMAZONAWS.COM.",
        ] {
            let mut config = ConnectionConfig {
                host: host.into(),
                ..Default::default()
            };
            for engine in [
                Engine::Mysql,
                Engine::AuroraMysql,
                Engine::Postgres,
                Engine::AuroraPostgres,
            ] {
                config.engine = engine;
                assert!(automatic_rds_roots(&config).is_some());
            }
            config.ca_file = "my-explicit-ca.pem".into();
            assert!(automatic_rds_roots(&config).is_none());
            config.ca_file.clear();
            config.tls = false;
            assert!(automatic_rds_roots(&config).is_none());
            config.tls = true;
            config.engine = Engine::Redis;
            assert!(automatic_rds_roots(&config).is_none());
        }
    }
    #[test]
    fn unrelated_lookalike_and_invalid_endpoints_never_gain_provider_roots() {
        for host in [
            "localhost",
            "127.0.0.1",
            "rds.amazonaws.com",
            "db.rds.amazonaws.com",
            "db.us-east-1.rds.amazonaws.com",
            "db.abc.us-east-1.rds.amazonaws.com.evil.test",
            "db.abc.us-east-1.evilrds.amazonaws.com",
            "db..us-east-1.rds.amazonaws.com",
            "db.abc.bad-region.rds.amazonaws.com",
            "db.abc.us-east-1.rds.amazonaws.com..",
            "db/abc.us-east-1.rds.amazonaws.com",
            "-db.abc.us-east-1.rds.amazonaws.com",
        ] {
            assert!(!is_rds_host(host), "unexpected provider endpoint: {host}");
        }
    }
    #[test]
    fn embedded_bundle_is_nonempty_public_pem_without_private_keys() {
        let pem = std::str::from_utf8(AWS_RDS_ROOTS).unwrap();
        assert!(AWS_RDS_ROOTS.len() < 256 * 1024);
        assert!(pem.matches("-----BEGIN CERTIFICATE-----").count() >= 30);
        assert_eq!(
            pem.matches("-----BEGIN CERTIFICATE-----").count(),
            pem.matches("-----END CERTIFICATE-----").count()
        );
        assert!(!pem.contains("PRIVATE KEY"));
    }
    #[test]
    fn documentdb_automatic_trust_is_scoped_and_explicit_ca_wins() {
        let mut config = ConnectionConfig {
            engine: Engine::Mongo,
            host: "db.cluster-xyz.sa-east-1.docdb.amazonaws.com".into(),
            ..Default::default()
        };
        assert!(automatic_aws_roots(&config).is_some());
        for host in [
            "db.cluster-xyz.sa-east-1.docdb.amazonaws.com.evil.test",
            "db..sa-east-1.docdb.amazonaws.com",
            "localhost",
            "127.0.0.1",
        ] {
            config.host = host.into();
            assert!(automatic_aws_roots(&config).is_none());
        }
        config.host = "db.cluster-xyz.sa-east-1.docdb.amazonaws.com".into();
        config.ca_file = "explicit.pem".into();
        assert!(automatic_aws_roots(&config).is_none());
        config.ca_file.clear();
        config.ca_pem = "explicit".into();
        assert!(automatic_aws_roots(&config).is_none());
        config.ca_pem = Default::default();
        config.tls = false;
        assert!(automatic_aws_roots(&config).is_none());
    }
    #[test]
    fn imported_ca_survives_original_file_removal_and_temporary_file_is_owned() {
        let source = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(source.path(), AWS_RDS_ROOTS).unwrap();
        let mut config = ConnectionConfig {
            ca_file: source.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        prepare_saved_certificate(&mut config).unwrap();
        drop(source);
        assert!(config.ca_file.is_empty());
        assert_eq!(config.ca_pem.as_bytes(), AWS_RDS_ROOTS);
        let restored: ConnectionConfig =
            serde_json::from_slice(&serde_json::to_vec(&config).unwrap()).unwrap();
        let bytes = material(&restored).unwrap().unwrap();
        assert_eq!(&*bytes, AWS_RDS_ROOTS);
        let temp = temporary_pem(&bytes).unwrap();
        let path = temp.path().to_path_buf();
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                temp.as_file().metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(temp);
        assert!(!path.exists());
        assert!(
            validate_pem(b"-----BEGIN PRIVATE KEY-----\nsecret\n-----END PRIVATE KEY-----")
                .is_err()
        );
        assert!(
            validate_pem(b"-----BEGIN CERTIFICATE-----\ninvalid!\n-----END CERTIFICATE-----")
                .is_err()
        );
        assert!(validate_pem(&vec![b'x'; MAX_CA_BYTES + 1]).is_err());
    }
}
