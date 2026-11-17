# AWS RDS roots

Public X.509 CA certificates published by Amazon Web Services for client trust
stores. Source and SHA256 are recorded in source.json; retrieved over verified
HTTPS on27/10/2026. Only subject==issuer roots were selected; no private keys,
application code, UI assets or intermediate certificates are included.

Applied only to eligible RDS SQL hosts with verified TLS and no explicit CA.
This does not install certificates in the operating system trust store.
The data ships with the executable; refreshing it requires an audited product
update from the official source. Review fingerprints and root validity before
replacing this file. Runtime never downloads or trusts a server-provided CA.
