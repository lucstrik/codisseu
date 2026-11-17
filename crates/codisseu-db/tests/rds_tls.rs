//! Opt-in read-only connection check. Credentials never enter test output.
use codisseu_db::{
    remote::{Cell, ConnectionConfig},
    remote_sql::SqlSession,
};
use std::{collections::BTreeMap, path::PathBuf};
use zeroize::Zeroizing;

fn credentials() -> (ConnectionConfig, Zeroizing<String>) {
    let path = std::env::var_os("CODISSEU_RDS_CREDENTIAL_FILE")
        .map(PathBuf::from)
        .expect("Select an explicit private credential file");
    let text = Zeroizing::new(std::fs::read_to_string(path).expect("Read credential file"));
    let mut fields = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Some(split) = line.find(['=', ':']) else {
            panic!("Invalid credential file format")
        };
        let key = line[..split].trim().to_ascii_lowercase();
        let value = line[split + 1..].trim();
        let value = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            &value[1..value.len() - 1]
        } else {
            value
        };
        fields.insert(key, Zeroizing::new(value.to_owned()));
    }
    let mut take = |key: &str| {
        fields
            .remove(key)
            .expect("Required credential field absent")
    };
    let host = take("host");
    let user = take("user");
    let password = take("senha");
    let port = take("port");
    let config = ConnectionConfig {
        id: "rds-readonly-check".into(),
        name: "RDS read-only check".into(),
        host: host.to_string(),
        username: user.to_string(),
        port: port.parse().expect("Invalid port"),
        tls: true,
        ca_file: String::new(),
        read_only: true,
        save_credentials: false,
        timeout_secs: 10,
        row_limit: 1,
        ..Default::default()
    };
    assert!(
        config.automatic_rds_ca(),
        "Only an explicitly selected AWS RDS SQL endpoint is supported"
    );
    (config, password)
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
#[ignore = "requires explicit CODISSEU_RDS_CREDENTIAL_FILE; TLS/auth and SELECT 1 only"]
fn automatic_rds_tls_authenticates_and_reads_one_constant() {
    let (config, password) = credentials();
    runtime().block_on(async {
        let mut session = SqlSession::connect(&config, &password)
            .await
            .unwrap_or_else(|_| panic!("RDS TLS/auth failed; private details suppressed"));
        let result = session
            .execute("SELECT 1 AS connection_ok")
            .await
            .unwrap_or_else(|_| {
                panic!("Read-only constant query failed; private details suppressed")
            });
        assert_eq!(result.rows, vec![vec![Cell::Number("1".into())]]);
        session
            .close()
            .await
            .unwrap_or_else(|_| panic!("Could not close the owned connection"));
    });
}

#[test]
#[ignore = "requires explicit CODISSEU_RDS_CREDENTIAL_FILE and CODISSEU_RDS_REJECT_CA_FILE"]
fn explicit_untrusted_regional_ca_is_not_silently_overridden() {
    let (mut config, password) = credentials();
    config.ca_file = std::env::var("CODISSEU_RDS_REJECT_CA_FILE")
        .expect("Select explicit untrusted regional CA");
    assert!(!config.automatic_rds_ca());
    runtime().block_on(async {
        match SqlSession::connect(&config, &password).await {
            Ok(mut session) => {
                let _ = session.close().await;
                panic!("Unexpectedly trusted the explicitly wrong CA")
            }
            Err(error) => assert!(
                error.contains("Certificado TLS"),
                "Expected certificate rejection; private details suppressed"
            ),
        }
    });
}

#[test]
#[ignore = "requires explicit CODISSEU_RDS_CREDENTIAL_FILE; verify certificate hostname rejection"]
fn trusted_provider_ca_does_not_allow_an_incorrect_server_hostname() {
    let (mut config, password) = credentials();
    runtime().block_on(async {
        let address = tokio::net::lookup_host((config.host.as_str(), config.port))
            .await
            .expect("Could not resolve selected endpoint")
            .next()
            .expect("Endpoint has no address");
        config.host = address.ip().to_string();
        config.ca_file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("certificates/aws-rds-roots.pem")
            .to_string_lossy()
            .into_owned();
        match SqlSession::connect(&config, &password).await {
            Ok(mut session) => {
                let _ = session.close().await;
                panic!("Unexpectedly accepted a hostname outside the certificate")
            }
            Err(error) => assert!(
                error.contains("Certificado TLS"),
                "Expected hostname rejection; private details suppressed"
            ),
        }
    });
}
