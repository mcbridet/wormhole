# Wormhole

**Wormhole** is a split-screen, serial terminal chat application designed for VT120 terminals (and emulators).

## Features

- **Serial Communication**: Optimized for VT100/VT120 terminals with split-screen UI.
- **P2P Chat**: Decentralized chat over UDP with automatic peer discovery (LAN/STUN).
- **Video Calling**: ASCII-art based video streaming from your webcam.
- **AI Assistant**: Integrated Google Gemini AI chat.
- **Webcam Snapshot Sharing**: Share images (converted to ASCII).

## Prerequisites

- Rust (implemented on v1.90)
- Linux (for `v4l` webcam support)
- A webcam (optional)
- A Google Gemini API key (optional)

## Configuration

Wormhole uses INI files for configuration. Copy `example.ini` to `wormhole.ini` and edit it to suit your needs.

```bash
cp example.ini wormhole.ini
nano wormhole.ini
```

### Key Configuration Options

- **[serial]**: Port and baud rate (default 9600).
- **[network]**: Node name, port, and peer list.
- **[webcam]**: Device path (e.g., `/dev/video0`) and FPS.
- **[gemini]**: API key and model selection.

## Usage

Run the application with your configuration file:

```bash
cargo run -- --config wormhole.ini
```