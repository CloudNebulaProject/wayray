# Audio & USB

> This page will be expanded as peripheral support is implemented.

## Audio

Audio is forwarded bidirectionally between server and client using the Opus codec.

### Output (Server -> Client)
Applications on the server produce audio, which is captured, encoded, and played through the client's speakers.

### Input (Client -> Server)
The client's microphone is captured, encoded, and made available as a virtual input device on the server.

## USB Forwarding

USB devices connected to the client can be forwarded to the server session.

### Supported Device Types
- Mass storage (flash drives, external drives)
- Printers
- Scanners
- Serial adapters

### Security
USB device forwarding can be restricted by device class at the server level. Administrators can allow or deny specific device types.
