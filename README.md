# Wormhole

Wormhole is a serial terminal application designed for VT100/VT220/VT340 terminals (and emulators). It combines retro-terminal aesthetics with modern networking, multimedia, and AI capabilities.

## Tabs

Wormhole is organized into tabs, navigated with the Tab key:

### 💬 Chat
Decentralized P2P chat over UDP with automatic peer discovery (LAN broadcast + STUN for NAT traversal).
- `/call <peer>` - Initiate a video call
- `/me <action>` - IRC-style action messages
- `/image` - Share a webcam snapshot
- `/who` - List online peers
- `/clear` - Clear chat history
- Mentions of your name trigger a terminal bell notification

### 📹 Call
ASCII-art or Sixel video calling with your webcam.
- VT100: ASCII block characters
- VT220: DRCS grayscale shading (4 brightness levels)
- VT340: Sixel graphics (configurable grayscale palette)
- Differential rendering for efficient updates over serial

### 🤖 AI
Chat with Google Gemini directly from your terminal.
- Configurable system prompt
- Streaming responses
- Plain text output optimized for hardware terminals

### 🎵 Tunes
Browse and play audio files from a configured directory.
- Supports MP3, WAV, FLAC, and OGG
- Play/pause controls
- Track duration display

## Features

- **Terminal Support**: VT100 (ASCII), VT220 (DRCS shading), VT340 (Sixel graphics)
- **132 Column Mode**: Wide display support for VT220+ terminals
- **Serial Optimization**: Differential rendering minimizes bandwidth usage
- **Peer Discovery**: Automatic LAN discovery with optional STUN/UPnP for internet connectivity
- **Scrollback**: Chat history with Page Up/Down navigation
- **Logging**: Optional disk logging of chat and AI conversations
- **Cross-compilation**: Builds for x86_64, aarch64 (Raspberry Pi 4/5), and armv7 (Raspberry Pi 2/3)

## Prerequisites

- Rust 1.90+
- Linux (V4L2 webcam) or macOS (AVFoundation webcam)
- A serial terminal or emulator (VT100/VT220/VT340 compatible)

### Optional
- Webcam (for video calls and image sharing)
- Google Gemini API key (for AI tab)
- Audio output device (for Tunes tab)

## Installation

### From Release
Download pre-built binaries from the [Releases](https://github.com/mcbridet/wormhole/releases) page.

### From Source
```bash
git clone https://github.com/mcbridet/wormhole.git
cd wormhole
cargo build --release
```

## Configuration

Copy `example.ini` and customize:

```bash
cp example.ini wormhole.ini
```

## Usage

```bash
cargo run --release -- --config wormhole.ini
```
