# Wormhole

Wormhole is a serial terminal chat application designed for VT220 terminals (and emulators). It combines retro-terminal aesthetics with modern networking and AI capabilities.

## Features

- **Serial Communication and TUI**: Optimized for VT120+ terminals. Assumes 80col for VT100 and 132col for VT220+.
- **DEC Character Set Support**: Support for DEC special graphics (VT100+) and DRCS mode (VT220+) for brightness shades. 
- **P2P Chat**: Decentralised chat over UDP with peer discovery (LAN/STUN).
- **Video Calling for some reason**: ASCII-art based video streaming from your webcam. No audio, sorry.
- **Differential Rendering**: Line-calculated diff rendering for efficient updates, makes video calls faster.
- **Buffer History**: Re-send recently sent messages.
- **Logs**: Tab logs are saved to disk.
- **AI Assistant**: Yep, there's a tab where you can talk to Gemini with a configurable system prompt, on your VTxxx.
- **Bugs**: Heaps of them, probably. Don't blame me if your old VT is the problem though.
## Prerequisites

- Rust (implemented on v1.90)
- Linux (for `v4l` webcam support)
- A webcam (optional)
- A Google Gemini API key (optional)
- Some terminal (real or virtual) compatible with VT1xx or VT2xx+ features

## Configuration

Wormhole uses INI files for configuration. Copy `example.ini` to `wormhole.ini` and edit it to suit your needs.

```bash
cp example.ini wormhole.ini
nano wormhole.ini
```

### Key Configuration Options

- **[terminal]**: Terminal mode (vt100 or vt200).
- **[serial]**: Port and baud rate (default 9600).
- **[network]**: Node name, port, and peer list.
- **[webcam]**: Device path (e.g., `/dev/video0`) and render FPS (keep low).
- **[gemini]**: API key and model selection.

## Usage

Run the application with your configuration file:

```bash
cargo run -- --config wormhole.ini
```
