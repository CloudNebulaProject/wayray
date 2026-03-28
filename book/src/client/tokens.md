# Token Setup

> This page will be expanded as token providers are implemented.

## Software Tokens

The simplest token type. A UUID is generated and stored on the client.

```bash
# Generate a new software token
wayray-client token generate
# Output: Token: a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

## Smart Card Tokens

Requires a PC/SC compatible smart card reader.

```bash
# List available smart card readers
wayray-client token list-readers

# Use smart card for session identity
wayray-client --token-type smartcard --server <host>:<port>
```

## NFC Tokens

Requires an NFC reader.

```bash
# Use NFC for session identity
wayray-client --token-type nfc --server <host>:<port>
```
