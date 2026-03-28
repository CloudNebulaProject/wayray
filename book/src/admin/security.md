# Security

> This page will be expanded as security features are implemented.

## Encryption

All WayRay traffic is encrypted via TLS 1.3 (mandatory in QUIC). There is no unencrypted mode.

## Authentication

- User authentication via PAM (supports LDAP, Kerberos, local accounts)
- Session binding via tokens
- Optional multi-factor authentication

## Client Security

- No data at rest on client devices
- No persistent storage of credentials
- Optional: lock session on token removal

## Network Security

- TLS 1.3 with strong cipher suites
- Certificate pinning supported
- Optional mutual TLS (client certificates)
