mod app;
mod config;
mod gemini;
mod graphics;
mod input;
mod log;
mod network;
mod serial;
mod terminal;
mod tunes;
mod webcam;

use app::App;
use chrono::Local;
use clap::Parser;
use config::Config;
use input::{EscapeParser, EscapeSequence, InputEvent, parse_byte};
use network::{Message, PEER_TIMEOUT, PeerEvent};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use terminal::{
    Tab, cleanup_split_screen, generate_waiting_for_peer_frame, init_split_screen_with_tabs,
    max_input_length, redraw_input, redraw_tab_bar, render_stream,
};
use webcam::{RawFrame, raw_frame_to_output};

#[derive(Parser, Debug)]
#[command(name = "wormhole")]
#[command(about = "A serial terminal chat application for VT220 terminals")]
struct Args {
    /// Path to the configuration file
    #[arg(short, long, default_value = "wormhole.ini")]
    config: PathBuf,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let args = Args::parse();

    // Show app info
    println!(
        "{} v{} - {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_AUTHORS")
    );

    let config = match Config::load(&args.config) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    // Show configuration
    println!();
    println!("Serial:");
    println!("  Port: {}", config.serial.port);
    println!("  Baud: {}", config.serial.baud_rate);
    println!();
    println!("Network:");
    println!("  Name: {}", config.network.name);
    println!("  Port: {}", config.network.port);
    if let Some(ref ip) = config.network.bind_ip {
        println!("  Bind IP: {}", ip);
    }
    println!(
        "  UPnP: {}",
        if config.network.upnp {
            "enabled"
        } else {
            "disabled"
        }
    );
    if config.network.peers.is_empty() {
        println!("  External Peers: (none configured)");
    } else {
        println!("  External Peers: {}", config.network.peers);
    }
    println!();
    println!("Webcam:");
    if let Some(ref device) = config.webcam.device {
        println!("  Device: {}", device);
        if config.webcam.fps > 0 {
            println!("  FPS: {}", config.webcam.fps);
        }
        if config.terminal.mode == "vt340" {
            println!("  Sixel Shades: {}", config.webcam.sixel_shades);
        }
    } else {
        println!("  Device: (not configured)");
    }
    println!();
    println!("Gemini AI:");
    if config.gemini.api_key.is_some() {
        println!("  Model: {}", config.gemini.model);
        if config.gemini.system_prompt.is_some() {
            println!("  System Prompt: (configured)");
        }
    } else {
        println!("  API Key: (not configured)");
    }
    println!();
    println!("Terminal:");
    println!("  Mode: {}", config.terminal.mode);
    if config.terminal.cols_132 {
        println!("  132 Columns: enabled");
    }
    println!();
    println!("Logging:");
    if let Some(ref dir) = config.logging.directory {
        println!("  Directory: {}", dir);
    } else {
        println!("  Directory: (not configured)");
    }
    println!();
    println!("Tunes:");
    if let Some(ref dir) = config.tunes.directory {
        println!("  Directory: {}", dir);
    } else {
        println!("  Directory: (not configured)");
    }
    println!();

    // Set up signal handler for clean shutdown
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl+C handler");

    // Initialize App
    let mut app = match App::new(config, running.clone()).await {
        Ok(app) => app,
        Err(e) => {
            eprintln!("Failed to initialize app: {}", e);
            std::process::exit(1);
        }
    };

    // Determine terminal width based on config
    let use_132_cols = app.config.terminal.cols_132;
    let width = if use_132_cols { 132 } else { 80 };

    // Main loop - handle serial I/O and network messages
    let max_input_len = max_input_length(&app.config.network.name, width);
    let mut serial_buf = [0u8; 256];
    let mut escape_parser = EscapeParser::new(); // Parser for escape sequences
    let mut last_reconnect_attempt = std::time::Instant::now();
    const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);

    // Calculate frame delay based on baud rate to avoid flooding the serial link
    // Frame size ~ 65x20 chars + overhead ~ 1500 bytes.
    let bytes_per_frame = 2000;
    let chars_per_sec = std::cmp::max(app.config.serial.baud_rate / 10, 1);
    let calculated_fps = (chars_per_sec as f64 / bytes_per_frame as f64).clamp(0.5, 30.0);

    let target_fps = if app.config.webcam.fps > 0 {
        app.config.webcam.fps as f64
    } else {
        calculated_fps
    };

    let frame_delay = Duration::from_secs_f64(1.0 / target_fps);
    let mut last_frame_time = std::time::Instant::now()
        .checked_sub(frame_delay)
        .unwrap_or_else(std::time::Instant::now);

    // Tunes status refresh timer (1 second for MM:SS display)
    let tunes_refresh_delay = Duration::from_secs(1);
    let mut last_tunes_refresh = std::time::Instant::now();

    // Main loop uses tokio::time::sleep to yield properly to the async runtime
    let loop_delay = Duration::from_millis(1);

    while app.running.load(Ordering::SeqCst) {
        // Sleep using tokio to properly yield to other tasks
        tokio::time::sleep(loop_delay).await;
        // Handle serial reconnection if disconnected
        if !app.serial.is_connected() {
            if last_reconnect_attempt.elapsed() >= RECONNECT_INTERVAL {
                last_reconnect_attempt = std::time::Instant::now();
                eprintln!("Attempting to reconnect to {}...", app.serial.port_path());
                match app.serial.reconnect() {
                    Ok(()) => {
                        eprintln!("Reconnected to serial port!");
                        // Reinitialize the terminal UI
                        let call_status = app.active_call.as_ref().map(|peer_name| {
                            format!("Call session with {}. Press Space to hang up.", peer_name)
                        });
                        let gemini_available = app.gemini_chat.is_some();

                        // Re-send DRCS init if needed
                        let use_drcs = app.config.terminal.mode == "vt220"
                            || app.config.terminal.mode == "vt340";
                        let _ = app
                            .serial
                            .write_str(&terminal::get_init_sequence(use_drcs, use_132_cols));

                        let tunes_available = app.tunes_available();
                        let _ = app.serial.write_str(&init_split_screen_with_tabs(
                            &app.config.network.name,
                            app.active_tab,
                            gemini_available,
                            tunes_available,
                            app.active_call.as_deref(),
                            call_status.as_deref(),
                            width,
                        ));
                        // Render the active buffer
                        match app.active_tab {
                            Tab::Chat => {
                                let _ = app.serial.write_str(&app.chat_buffer.render());
                                let _ = app.serial.write_str(&redraw_input(
                                    &app.config.network.name,
                                    &app.line_buffer,
                                    app.input_cursor,
                                    width,
                                ));
                            }
                            Tab::Gemini => {
                                let _ = app.serial.write_str(&app.ai_buffer.render());
                                let _ = app.serial.write_str(&redraw_input(
                                    &app.config.network.name,
                                    &app.line_buffer,
                                    app.input_cursor,
                                    width,
                                ));
                            }
                            Tab::Tunes => {
                                if let Some(ref tunes) = app.tunes_state {
                                    let _ = app.serial.write_str(&tunes.render());
                                }
                            }
                            Tab::Call => {}
                        }
                    }
                    Err(_) => {
                        // Still disconnected, wait and try again
                    }
                }
            }
            // Yield while disconnected
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Still process network messages while disconnected
            while let Ok(msg) = app.net_rx.try_recv() {
                match msg {
                    Message::Chat { from, text } => {
                        let timestamp = Local::now().format("%I:%M%p");

                        // Check if this is an image message
                        if text.starts_with("[IMAGE]\n") {
                            app.push_chat(format!("[{}] {} shared an image:", timestamp, from));
                            for line in text.strip_prefix("[IMAGE]\n").unwrap_or(&text).lines() {
                                app.push_chat(line.to_string());
                            }
                        } else {
                            let formatted = format!("[{}] {}: {}", timestamp, from, text);
                            app.push_chat(formatted);
                        }
                        app.chat_buffer.scroll_to_bottom();
                    }
                    Message::StreamFrame { from, .. } => {
                        // Legacy: ignore pre-rendered StreamFrame from older peers
                        // Peers should upgrade to use VideoFrame for cross-terminal compatibility
                        eprintln!("Received legacy StreamFrame from {} (ignored)", from);
                    }
                    Message::VideoFrame {
                        from,
                        width,
                        height,
                        pixels,
                    } => {
                        app.current_video_frame = Some((
                            from,
                            RawFrame {
                                width,
                                height,
                                pixels,
                            },
                        ));
                    }
                    _ => {}
                }
            }
            while let Ok(event) = app.peer_event_rx.try_recv() {
                let timestamp = Local::now().format("%I:%M%p");
                let msg = match event {
                    PeerEvent::Joined { name, addr } => {
                        app.net_node.add_peer(name.clone(), addr);
                        format!("[{}] *** {} has joined ***", timestamp, name)
                    }
                    PeerEvent::Left { name, addr } => {
                        app.net_node.remove_peer(addr);
                        format!("[{}] *** {} has left ***", timestamp, name)
                    }
                };
                app.push_chat(msg);
            }
            continue;
        }

        // Prune stale peers periodically (allows reconnection after timeout)
        let timed_out_peers = app.net_node.prune_peers(PEER_TIMEOUT);
        for peer in timed_out_peers {
            let timestamp = Local::now().format("%I:%M%p");
            let msg = format!("[{}] *** {} has timed out ***", timestamp, peer.name);
            app.push_chat(msg);
            if app.active_tab == Tab::Chat {
                let _ = app.serial.write_str(&app.chat_buffer.render());
            }
        }

        // Check for call timeout (tighter timeout than general peer timeout)
        if let Some(last_packet) = app.call_last_packet {
            let timeout = if app.call_connected {
                Duration::from_secs(5)
            } else {
                Duration::from_secs(30)
            };

            if last_packet.elapsed() > timeout {
                // Don't timeout self-calls
                let is_self_call = app.active_call.as_deref() == Some(&app.config.network.name);

                if !is_self_call && let Some(peer_name) = app.active_call.take() {
                    let timestamp = Local::now().format("%I:%M%p");
                    app.push_chat(format!(
                        "[{}] *** Call with {} timed out ***",
                        timestamp, peer_name
                    ));
                    app.last_rendered_frame = None;
                    app.call_last_packet = None;
                    app.call_connected = false;

                    // Redraw UI if needed
                    if app.active_tab == Tab::Call {
                        // Switch back to Chat
                        app.active_tab = Tab::Chat;
                        let gemini_available = app.gemini_chat.is_some();
                        let tunes_available = app.tunes_available();
                        let _ = app.serial.write_str(&init_split_screen_with_tabs(
                            &app.config.network.name,
                            app.active_tab,
                            gemini_available,
                            tunes_available,
                            app.active_call.as_deref(),
                            None,
                            width,
                        ));
                        let _ = app.serial.write_str(&app.chat_buffer.render());
                        let _ = app.serial.write_str(&redraw_input(
                            &app.config.network.name,
                            &app.line_buffer,
                            app.input_cursor,
                            width,
                        ));
                    } else {
                        // Just update the tab bar
                        let gemini_available = app.gemini_chat.is_some();
                        let tunes_available = app.tunes_available();
                        let _ = app.serial.write_str(&redraw_tab_bar(
                            app.active_tab,
                            gemini_available,
                            tunes_available,
                            app.active_call.as_deref(),
                            width,
                        ));
                    }
                }
            }
        }

        // Check for peer events (join/leave)
        while let Ok(event) = app.peer_event_rx.try_recv() {
            let timestamp = Local::now().format("%I:%M%p");
            let msg = match event {
                PeerEvent::Joined { name, addr } => {
                    app.net_node.add_peer(name.clone(), addr);
                    format!("[{}] *** {} has joined ***", timestamp, name)
                }
                PeerEvent::Left { name, addr } => {
                    app.net_node.remove_peer(addr);
                    format!("[{}] *** {} has left ***", timestamp, name)
                }
            };
            app.push_chat(msg);
            if app.active_tab == Tab::Chat {
                let _ = app.serial.write_str(&app.chat_buffer.render());
            }
        }

        // Check for discovered peers
        while let Ok(peer) = app.discovery_rx.try_recv() {
            // Check if this is a peer we already know and is still active
            if app.net_node.has_peer(peer.addr, PEER_TIMEOUT) {
                // Update last_seen for active peers
                app.net_node.touch_peer(peer.addr);
                continue;
            }

            // Skip peers who recently sent a Leave message (grace period)
            if app.net_node.recently_left(peer.addr) {
                continue;
            }

            // Check if this is a peer we've seen before (reconnecting)
            let is_reconnect = app.net_node.knows_peer(peer.addr);

            // Add peer and send join message
            if let Err(e) = app.net_node.connect_to_peer(peer.addr).await {
                eprintln!("Failed to connect to peer: {}", e);
            } else {
                if is_reconnect {
                    eprintln!("Peer reconnected: {} at {}", peer.name, peer.addr);
                } else {
                    eprintln!("Discovered peer: {} at {}", peer.name, peer.addr);
                }
                app.net_node.add_peer(peer.name.clone(), peer.addr);
            }
        }

        // Check for incoming network messages - drain all pending
        let mut had_messages = false;
        while let Ok(msg) = app.net_rx.try_recv() {
            // Update call timeout if message is from active peer
            if let Some(peer_name) = &app.active_call {
                let from_peer = match &msg {
                    Message::Chat { from, .. } => Some(from),
                    Message::StreamFrame { from, .. } => Some(from),
                    Message::VideoFrame { from, .. } => Some(from),
                    Message::CallRequest { from } => Some(from),
                    Message::CallHangup { from } => Some(from),
                    _ => None,
                };

                if let Some(from) = from_peer
                    && from == peer_name
                {
                    app.call_last_packet = Some(std::time::Instant::now());
                    app.call_connected = true;
                }
            }

            match msg {
                Message::Chat { from, text } => {
                    let timestamp = Local::now().format("%I:%M%p");

                    // Check if our name is mentioned in the message (case-insensitive)
                    let my_name = &app.config.network.name;
                    if from != *my_name && text.to_lowercase().contains(&my_name.to_lowercase()) {
                        let _ = app.serial.write_str("\x07");
                    }

                    // Check if this is an image message
                    if text.starts_with("[IMAGE]\n") {
                        // Add header
                        app.push_chat(format!("[{}] {} shared an image:", timestamp, from));
                        // Add each line of the ASCII art
                        for line in text.strip_prefix("[IMAGE]\n").unwrap_or(&text).lines() {
                            app.push_chat(line.to_string());
                        }
                    } else if text.starts_with("\x01ACTION ") {
                        // IRC-style /me action
                        let action = text.strip_prefix("\x01ACTION ").unwrap_or("");
                        let formatted = format!("[{}] * {} {}", timestamp, from, action);
                        app.push_chat(formatted);
                    } else {
                        // Regular chat message
                        let formatted = format!("[{}] {}: {}", timestamp, from, text);
                        app.push_chat(formatted);
                    }
                    app.chat_buffer.scroll_to_bottom();
                    had_messages = true;
                }
                Message::CallRequest { from } => {
                    let is_busy = if let Some(current_peer) = &app.active_call {
                        current_peer != &from
                    } else {
                        false
                    };

                    if is_busy {
                        // We are busy, reject the call
                        let msg = Message::CallReject {
                            from: app.config.network.name.clone(),
                        };
                        if let Some(peer) = app.net_node.peers().iter().find(|p| p.name == from)
                            && let Err(e) =
                                futures::executor::block_on(app.net_node.send_to(&msg, peer.addr))
                        {
                            eprintln!("Failed to send call rejection: {}", e);
                        }
                    } else {
                        let timestamp = Local::now().format("%I:%M%p");

                        // If we are already calling them, this is an answer
                        if app.active_call.as_deref() == Some(&from) {
                            let msg =
                                format!("[{}] *** Call connected with {} ***", timestamp, from);
                            app.push_chat(msg);
                            app.call_connected = true;
                        } else {
                            let msg = format!(
                                "[{}] *** {} has initiated a call with you ***",
                                timestamp, from
                            );
                            app.push_chat(msg);
                            // Ring the bell (3 times for a ringing effect)
                            let _ = app.serial.write_str("\x07\x07\x07");
                        }

                        app.chat_buffer.scroll_to_bottom();
                        had_messages = true;
                    }
                }
                Message::CallReject { from } => {
                    let timestamp = Local::now().format("%I:%M%p");
                    let msg = format!("[{}] *** {} is busy ***", timestamp, from);
                    app.push_chat(msg);
                    app.chat_buffer.scroll_to_bottom();
                    had_messages = true;

                    // If we are trying to call this person, hang up
                    if let Some(current_peer) = &app.active_call
                        && current_peer == &from
                    {
                        app.active_call = None;
                        app.last_rendered_frame = None;
                        app.call_last_packet = None;
                        app.call_connected = false;

                        // Stop webcam
                        if let Some(cam) = &app.webcam {
                            cam.stop().await;
                        }

                        // Switch back to Chat
                        if app.active_tab == Tab::Call {
                            app.active_tab = Tab::Chat;
                            let gemini_available = app.gemini_chat.is_some();
                            let tunes_available = app.tunes_available();
                            let _ = app.serial.write_str(&init_split_screen_with_tabs(
                                &app.config.network.name,
                                app.active_tab,
                                gemini_available,
                                tunes_available,
                                app.active_call.as_deref(),
                                None,
                                width,
                            ));
                            let _ = app.serial.write_str(&app.chat_buffer.render());
                            let _ = app.serial.write_str(&redraw_input(
                                &app.config.network.name,
                                &app.line_buffer,
                                app.input_cursor,
                                width,
                            ));
                        } else {
                            // Just update the tab bar
                            let gemini_available = app.gemini_chat.is_some();
                            let tunes_available = app.tunes_available();
                            let _ = app.serial.write_str(&redraw_tab_bar(
                                app.active_tab,
                                gemini_available,
                                tunes_available,
                                app.active_call.as_deref(),
                                width,
                            ));
                        }
                    }
                }
                Message::CallHangup { from } => {
                    let timestamp = Local::now().format("%I:%M%p");
                    let msg = format!("[{}] *** {} hung up ***", timestamp, from);
                    app.push_chat(msg);
                    app.chat_buffer.scroll_to_bottom();
                    had_messages = true;

                    // If we are in a call with this person, hang up
                    if let Some(current_peer) = &app.active_call
                        && current_peer == &from
                    {
                        app.active_call = None;
                        app.last_rendered_frame = None;
                        app.call_last_packet = None;
                        app.call_connected = false;

                        // Stop webcam
                        if let Some(cam) = &app.webcam {
                            cam.stop().await;
                        }

                        // If we were in the Call tab, switch back to Chat
                        if app.active_tab == Tab::Call {
                            app.active_tab = Tab::Chat;
                            let gemini_available = app.gemini_chat.is_some();
                            let tunes_available = app.tunes_available();
                            let _ = app.serial.write_str(&init_split_screen_with_tabs(
                                &app.config.network.name,
                                app.active_tab,
                                gemini_available,
                                tunes_available,
                                app.active_call.as_deref(),
                                None,
                                width,
                            ));
                            let _ = app.serial.write_str(&app.chat_buffer.render());
                            let _ = app.serial.write_str(&redraw_input(
                                &app.config.network.name,
                                &app.line_buffer,
                                app.input_cursor,
                                width,
                            ));
                        } else {
                            // Just update the tab bar
                            let gemini_available = app.gemini_chat.is_some();
                            let tunes_available = app.tunes_available();
                            let _ = app.serial.write_str(&redraw_tab_bar(
                                app.active_tab,
                                gemini_available,
                                tunes_available,
                                app.active_call.as_deref(),
                                width,
                            ));
                        }
                    }
                }
                Message::StreamFrame { from, .. } => {
                    // Legacy: ignore pre-rendered StreamFrame from older peers
                    eprintln!("Received legacy StreamFrame from {} (ignored)", from);
                }
                Message::VideoFrame {
                    from,
                    width: w,
                    height: h,
                    pixels,
                } => {
                    app.current_video_frame = Some((
                        from,
                        RawFrame {
                            width: w,
                            height: h,
                            pixels,
                        },
                    ));
                }
                _ => {}
            }
        }
        // Render once after processing all messages
        if had_messages
            && app.active_tab == Tab::Chat
            && let Err(e) = app.serial.write_str(&app.chat_buffer.render())
        {
            eprintln!("Serial write error: {}", e);
            break;
        }

        // Handle Call/Video logic
        // We process video if we are in the Call tab OR if we have an active call (background processing)
        if (app.active_tab == Tab::Call || app.active_call.is_some())
            && last_frame_time.elapsed() >= frame_delay
        {
            last_frame_time = std::time::Instant::now();
            let mut frame_to_render: Option<Vec<String>> = None;
            let mut sender_name = String::new();

            // Get render mode for local display
            let render_mode = webcam::RenderMode::from_terminal_mode(
                &app.config.terminal.mode,
                app.config.webcam.sixel_shades,
            );

            // Try to capture from webcam if available
            let mut local_raw_frame: Option<RawFrame> = None;
            if let Some(cam) = &app.webcam {
                match cam.capture_raw_frame(width).await {
                    Ok(raw_frame) => {
                        local_raw_frame = Some(raw_frame.clone());

                        // Only transmit if we are in a call with a remote peer
                        if let Some(target_name) = &app.active_call
                            && target_name != &app.config.network.name
                        {
                            // Find the peer address
                            let target_addr = app
                                .net_node
                                .peers()
                                .iter()
                                .find(|p| p.name == *target_name)
                                .map(|p| p.addr);

                            if let Some(addr) = target_addr {
                                // Send raw frame data - receiver will render according to their terminal
                                let msg = Message::VideoFrame {
                                    from: app.config.network.name.clone(),
                                    width: raw_frame.width,
                                    height: raw_frame.height,
                                    pixels: raw_frame.pixels,
                                };

                                if let Err(e) =
                                    futures::executor::block_on(app.net_node.send_to(&msg, addr))
                                {
                                    eprintln!("Failed to send video frame: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Webcam capture error: {}", e);
                    }
                }
            }

            // Only render if we are actually looking at the Call tab
            if app.active_tab == Tab::Call {
                // Determine what to render
                // 1. If we are calling someone, try to show their video
                if let Some(peer_name) = &app.active_call {
                    if let Some((from, raw_frame)) = &app.current_video_frame
                        && from == peer_name
                    {
                        // Render received raw frame according to OUR terminal mode
                        let lines = raw_frame_to_output(
                            raw_frame,
                            render_mode,
                            app.config.webcam.sixel_shades,
                        );
                        frame_to_render = Some(lines);
                        sender_name = from.clone();
                    }

                    // 2. If we haven't found their video yet, and we are calling "yourself", show local video
                    if frame_to_render.is_none()
                        && peer_name == &app.config.network.name
                        && let Some(raw_frame) = &local_raw_frame
                    {
                        let lines = raw_frame_to_output(
                            raw_frame,
                            render_mode,
                            app.config.webcam.sixel_shades,
                        );
                        frame_to_render = Some(lines);
                        sender_name = app.config.network.name.clone();
                    }

                    // 3. If still no frame, show the "waiting for peer" placeholder
                    if frame_to_render.is_none() {
                        frame_to_render = Some(generate_waiting_for_peer_frame(peer_name));
                        sender_name = peer_name.clone();
                    }
                }

                // 3. Fallback: If we still have nothing to render, show local video (mirror)
                //    ONLY if we are NOT in a call with someone else (to avoid showing self when waiting for peer)
                //    OR if we have received a frame from someone else (passive watching)
                if frame_to_render.is_none() {
                    if let Some((from, raw_frame)) = &app.current_video_frame {
                        let lines = raw_frame_to_output(
                            raw_frame,
                            render_mode,
                            app.config.webcam.sixel_shades,
                        );
                        frame_to_render = Some(lines);
                        sender_name = from.clone();
                    } else if app.active_call.is_none() {
                        // Only show mirror if not in a call
                        if let Some(raw_frame) = &local_raw_frame {
                            let lines = raw_frame_to_output(
                                raw_frame,
                                render_mode,
                                app.config.webcam.sixel_shades,
                            );
                            frame_to_render = Some(lines);
                            sender_name = app.config.network.name.clone();
                        }
                    }
                }

                // Render if we have a frame
                if let Some(lines) = frame_to_render {
                    let (rendered, frame) = render_stream(
                        &sender_name,
                        &lines,
                        app.last_rendered_frame.as_ref(),
                        width,
                    );
                    // Update stats with actual bytes sent (factors in differential rendering savings)
                    app.stats_bytes_sent += rendered.len();
                    app.stats_frames_rendered += 1;

                    if let Err(e) = app.serial.write_str(&rendered) {
                        eprintln!("Serial write error in Call tab: {}", e);
                    }
                    app.last_rendered_frame = Some(frame);
                }
            }

            // Periodic stats logging
            if app.stats_last_check.elapsed() >= Duration::from_secs(5) {
                let elapsed = app.stats_last_check.elapsed().as_secs_f64();
                let fps = app.stats_frames_rendered as f64 / elapsed;
                let kbps = (app.stats_bytes_sent as f64 / 1024.0) / elapsed;

                eprintln!("[Call Stats] FPS: {:.1}, BW: {:.1} KB/s", fps, kbps);

                app.stats_last_check = std::time::Instant::now();
                app.stats_frames_rendered = 0;
                app.stats_bytes_sent = 0;
            }
        }

        // Refresh tunes status display periodically when playing
        if app.active_tab == Tab::Tunes
            && last_tunes_refresh.elapsed() >= tunes_refresh_delay
            && let Some(ref tunes) = app.tunes_state
            && tunes.is_active()
        {
            last_tunes_refresh = std::time::Instant::now();
            let _ = app.serial.write_str(&tunes.render());
        }

        // Check for serial input
        match app.serial.read(&mut serial_buf) {
            Ok(0) => {
                // No data available - the loop interval already prevents busy-looping
            }
            Ok(n) => {
                // Process input character by character
                for &byte in &serial_buf[..n] {
                    // Handle escape sequences in progress
                    if escape_parser.is_parsing() {
                        if let Some(seq) = escape_parser.feed(byte) {
                            match seq {
                                EscapeSequence::PageUp => {
                                    // Page Up - scroll up (on active buffer) or page up in tunes
                                    if app.active_tab == Tab::Tunes {
                                        if let Some(ref mut tunes) = app.tunes_state {
                                            tunes.page_up();
                                            let _ = app.serial.write_str(&tunes.render());
                                        }
                                    } else {
                                        let active_buffer = if app.active_tab == Tab::Chat {
                                            &mut app.chat_buffer
                                        } else {
                                            &mut app.ai_buffer
                                        };
                                        active_buffer.scroll_up(10);
                                        let _ = app.serial.write_str(&active_buffer.render());
                                    }
                                }
                                EscapeSequence::PageDown => {
                                    // Page Down - scroll down (on active buffer) or page down in tunes
                                    if app.active_tab == Tab::Tunes {
                                        if let Some(ref mut tunes) = app.tunes_state {
                                            tunes.page_down();
                                            let _ = app.serial.write_str(&tunes.render());
                                        }
                                    } else {
                                        let active_buffer = if app.active_tab == Tab::Chat {
                                            &mut app.chat_buffer
                                        } else {
                                            &mut app.ai_buffer
                                        };
                                        active_buffer.scroll_down(10);
                                        let _ = app.serial.write_str(&active_buffer.render());
                                    }
                                }
                                EscapeSequence::ArrowUp => {
                                    // Up Arrow - navigate tunes or history previous
                                    if app.active_tab == Tab::Tunes {
                                        if let Some(ref mut tunes) = app.tunes_state {
                                            tunes.move_up();
                                            let _ = app.serial.write_str(&tunes.render());
                                        }
                                    } else if app.active_tab != Tab::Call
                                        && !app.ai_processing
                                        && !app.input_history.is_empty()
                                    {
                                        let new_index = match app.history_index {
                                            Some(i) => {
                                                if i > 0 {
                                                    i - 1
                                                } else {
                                                    0
                                                }
                                            }
                                            None => app.input_history.len() - 1,
                                        };

                                        app.history_index = Some(new_index);
                                        app.line_buffer = app.input_history[new_index].clone();
                                        app.input_cursor = app.line_buffer.len();
                                        let _ = app.serial.write_str(&redraw_input(
                                            &app.config.network.name,
                                            &app.line_buffer,
                                            app.input_cursor,
                                            width,
                                        ));
                                    }
                                }
                                EscapeSequence::ArrowDown => {
                                    // Down Arrow - navigate tunes or history next
                                    if app.active_tab == Tab::Tunes {
                                        if let Some(ref mut tunes) = app.tunes_state {
                                            tunes.move_down();
                                            let _ = app.serial.write_str(&tunes.render());
                                        }
                                    } else if app.active_tab != Tab::Call
                                        && !app.ai_processing
                                        && let Some(i) = app.history_index
                                    {
                                        if i + 1 >= app.input_history.len() {
                                            // End of history, clear input
                                            app.history_index = None;
                                            app.line_buffer.clear();
                                            app.input_cursor = 0;
                                        } else {
                                            let new_index = i + 1;
                                            app.history_index = Some(new_index);
                                            app.line_buffer = app.input_history[new_index].clone();
                                            app.input_cursor = app.line_buffer.len();
                                        }
                                        let _ = app.serial.write_str(&redraw_input(
                                            &app.config.network.name,
                                            &app.line_buffer,
                                            app.input_cursor,
                                            width,
                                        ));
                                    }
                                }
                                EscapeSequence::ArrowRight => {
                                    // Right Arrow - Move Cursor Right
                                    if app.active_tab != Tab::Call
                                        && !app.ai_processing
                                        && app.input_cursor < app.line_buffer.len()
                                    {
                                        app.input_cursor += 1;
                                        let _ = app.serial.write_str(&redraw_input(
                                            &app.config.network.name,
                                            &app.line_buffer,
                                            app.input_cursor,
                                            width,
                                        ));
                                    }
                                }
                                EscapeSequence::ArrowLeft => {
                                    // Left Arrow - Move Cursor Left
                                    if app.active_tab != Tab::Call
                                        && !app.ai_processing
                                        && app.input_cursor > 0
                                    {
                                        app.input_cursor -= 1;
                                        let _ = app.serial.write_str(&redraw_input(
                                            &app.config.network.name,
                                            &app.line_buffer,
                                            app.input_cursor,
                                            width,
                                        ));
                                    }
                                }
                                EscapeSequence::Unknown => {
                                    // Unknown sequence, ignore
                                }
                            }
                        }
                        continue;
                    }

                    // Parse the byte into an input event
                    match parse_byte(byte) {
                        InputEvent::EscapeStart => {
                            // Start of escape sequence
                            escape_parser.feed(byte);
                        }
                        InputEvent::Enter => {
                            if app.ai_processing {
                                continue;
                            }

                            // Handle Enter for tabs that don't use line buffer
                            if app.active_tab == Tab::Tunes {
                                if let Some(ref mut tunes) = app.tunes_state {
                                    if let Err(e) = tunes.play_selected() {
                                        eprintln!("Failed to play: {}", e);
                                    }
                                    let _ = app.serial.write_str(&tunes.render());
                                }
                                continue;
                            }

                            if app.active_tab == Tab::Call {
                                // Call tab has no Enter action
                                continue;
                            }

                            if !app.line_buffer.is_empty() {
                                let text = app.line_buffer.clone();

                                // Add to history
                                if app.input_history.last() != Some(&text) {
                                    app.input_history.push(text.clone());
                                    if app.input_history.len() > 25 {
                                        app.input_history.remove(0);
                                    }
                                }
                                app.history_index = None;
                                app.line_buffer.clear();
                                app.input_cursor = 0;

                                // Redraw empty input line first
                                if app.active_tab != Tab::Call {
                                    let _ = app.serial.write_str(&redraw_input(
                                        &app.config.network.name,
                                        "",
                                        0,
                                        width,
                                    ));
                                }

                                // Handle input based on active tab
                                match app.active_tab {
                                    Tab::Chat => {
                                        // P2P Chat tab - handle commands and messages
                                        if text.starts_with('/') {
                                            if text.starts_with("/me ") {
                                                let action =
                                                    text.strip_prefix("/me ").unwrap_or("");
                                                let timestamp = Local::now().format("%I:%M%p");
                                                let formatted = format!(
                                                    "[{}] * {} {}",
                                                    timestamp, app.config.network.name, action
                                                );
                                                app.push_chat(formatted);
                                                app.chat_buffer.scroll_to_bottom();
                                                let _ =
                                                    app.serial.write_str(&app.chat_buffer.render());

                                                // Broadcast to peers
                                                let action_msg = format!("\x01ACTION {}", action);
                                                if let Err(e) = futures::executor::block_on(
                                                    app.net_node.send_chat(&action_msg),
                                                ) {
                                                    eprintln!("Failed to send action: {}", e);
                                                }
                                            } else {
                                                match text.as_str() {
                                                    "/image" => {
                                                        // Capture webcam snapshot
                                                        let timestamp =
                                                            Local::now().format("%I:%M%p");
                                                        let render_mode =
                                                            webcam::RenderMode::from_terminal_mode(
                                                                &app.config.terminal.mode,
                                                                app.config.webcam.sixel_shades,
                                                            );

                                                        let result = if let Some(cam) = &app.webcam
                                                        {
                                                            if let Some(device) =
                                                                &app.config.webcam.device
                                                            {
                                                                cam.take_snapshot(
                                                                    device.clone(),
                                                                    render_mode,
                                                                    width,
                                                                )
                                                                .await
                                                            } else {
                                                                Err(webcam::WebcamError::NotConfigured)
                                                            }
                                                        } else {
                                                            // Fallback if app.webcam is None (e.g. initialization failed or not configured)
                                                            webcam::capture_ascii_snapshot(
                                                                app.config.webcam.device.as_deref(),
                                                                render_mode,
                                                                width,
                                                            )
                                                        };

                                                        match result {
                                                            Ok(lines) => {
                                                                // Add header
                                                                app.push_chat(format!(
                                                                    "[{}] {} shared an image:",
                                                                    timestamp,
                                                                    app.config.network.name
                                                                ));
                                                                // Add each line of the ASCII art
                                                                for line in &lines {
                                                                    app.push_chat(line.clone());
                                                                }
                                                                app.chat_buffer.scroll_to_bottom();
                                                                let _ = app.serial.write_str(
                                                                    &app.chat_buffer.render(),
                                                                );

                                                                // Also send to peers as multi-line message
                                                                let img_msg = format!(
                                                                    "[IMAGE]\n{}",
                                                                    lines.join("\n")
                                                                );
                                                                if let Err(e) =
                                                                    futures::executor::block_on(
                                                                        app.net_node
                                                                            .send_chat(&img_msg),
                                                                    )
                                                                {
                                                                    eprintln!(
                                                                        "Failed to send image: {}",
                                                                        e
                                                                    );
                                                                }
                                                            }
                                                            Err(e) => {
                                                                let err_msg = format!(
                                                                    "[{}] *** Webcam error: {} ***",
                                                                    timestamp, e
                                                                );
                                                                app.push_chat(err_msg);
                                                                app.chat_buffer.scroll_to_bottom();
                                                                let _ = app.serial.write_str(
                                                                    &app.chat_buffer.render(),
                                                                );
                                                            }
                                                        }
                                                    }
                                                    "/help" => {
                                                        let timestamp =
                                                            Local::now().format("%I:%M%p");
                                                        app.push_chat(format!("[{}] *** /clear, /who, /image, /me <action>, /call <peer> ***", timestamp));
                                                        app.chat_buffer.scroll_to_bottom();
                                                        let _ = app
                                                            .serial
                                                            .write_str(&app.chat_buffer.render());
                                                    }
                                                    "/clear" => {
                                                        app.chat_buffer.clear();
                                                        let _ = app
                                                            .serial
                                                            .write_str(&app.chat_buffer.render());
                                                    }
                                                    "/who" => {
                                                        let timestamp =
                                                            Local::now().format("%I:%M%p");
                                                        let peers = app.net_node.peers();
                                                        if peers.is_empty() {
                                                            app.push_chat(format!(
                                                                "[{}] *** No peers connected ***",
                                                                timestamp
                                                            ));
                                                        } else {
                                                            let peer_count = peers.len();
                                                            let peer_info: Vec<_> = peers
                                                                .iter()
                                                                .map(|p| {
                                                                    format!(
                                                                        "  - {} ({})",
                                                                        p.name, p.addr
                                                                    )
                                                                })
                                                                .collect();
                                                            app.push_chat(format!(
                                                                "[{}] *** Connected Peers ({}) ***",
                                                                timestamp, peer_count
                                                            ));
                                                            for info in peer_info {
                                                                app.push_chat(info);
                                                            }
                                                        }
                                                        app.chat_buffer.scroll_to_bottom();
                                                        let _ = app
                                                            .serial
                                                            .write_str(&app.chat_buffer.render());
                                                    }
                                                    _ => {
                                                        if text.to_lowercase().starts_with("/call ")
                                                        {
                                                            let peer_name = text[6..].trim();
                                                            if !peer_name.is_empty() {
                                                                // Check if peer exists (or is self)
                                                                let peer_exists = peer_name
                                                                    == app.config.network.name
                                                                    || app
                                                                        .net_node
                                                                        .peers()
                                                                        .iter()
                                                                        .any(|p| {
                                                                            p.name == peer_name
                                                                        });

                                                                if peer_exists {
                                                                    // Send CallRequest if calling a remote peer
                                                                    if peer_name
                                                                        != app.config.network.name
                                                                        && let Some(peer) = app
                                                                            .net_node
                                                                            .peers()
                                                                            .iter()
                                                                            .find(|p| {
                                                                                p.name == peer_name
                                                                            })
                                                                    {
                                                                        let msg =
                                                                            Message::CallRequest {
                                                                                from: app
                                                                                    .config
                                                                                    .network
                                                                                    .name
                                                                                    .clone(),
                                                                            };
                                                                        if let Err(e) = futures::executor::block_on(app.net_node.send_to(&msg, peer.addr)) {
                                                                            eprintln!("Failed to send call request: {}", e);
                                                                        }
                                                                    }

                                                                    app.active_call =
                                                                        Some(peer_name.to_string());
                                                                    app.call_last_packet = Some(
                                                                        std::time::Instant::now(),
                                                                    );
                                                                    app.active_tab = Tab::Call;
                                                                    app.last_rendered_frame = None;

                                                                    // Start webcam
                                                                    if let Some(cam) = &app.webcam {
                                                                        cam.start().await;
                                                                    }

                                                                    // Redraw UI
                                                                    let status = format!(
                                                                        "Call session with {}. Press Space to hang up.",
                                                                        peer_name
                                                                    );
                                                                    let gemini_available =
                                                                        app.gemini_chat.is_some();
                                                                    let tunes_available =
                                                                        app.tunes_available();
                                                                    let _ = app.serial.write_str(&init_split_screen_with_tabs(&app.config.network.name, app.active_tab, gemini_available, tunes_available, app.active_call.as_deref(), Some(&status), width));
                                                                } else {
                                                                    let timestamp = Local::now()
                                                                        .format("%I:%M%p");
                                                                    app.push_chat(format!("[{}] *** Peer '{}' not found ***", timestamp, peer_name));
                                                                    app.chat_buffer
                                                                        .scroll_to_bottom();
                                                                    let _ = app.serial.write_str(
                                                                        &app.chat_buffer.render(),
                                                                    );
                                                                }
                                                            }
                                                        } else {
                                                            let timestamp =
                                                                Local::now().format("%I:%M%p");
                                                            app.push_chat(format!(
                                                                "[{}] *** Unknown command: {} ***",
                                                                timestamp, text
                                                            ));
                                                            app.chat_buffer.scroll_to_bottom();
                                                            let _ = app.serial.write_str(
                                                                &app.chat_buffer.render(),
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        } else {
                                            // Regular chat message
                                            let timestamp = Local::now().format("%I:%M%p");
                                            let our_msg = format!(
                                                "[{}] {}: {}",
                                                timestamp, app.config.network.name, text
                                            );
                                            app.push_chat(our_msg);
                                            app.chat_buffer.scroll_to_bottom();
                                            let _ = app.serial.write_str(&app.chat_buffer.render());

                                            // Broadcast to peers
                                            if let Err(e) = futures::executor::block_on(
                                                app.net_node.send_chat(&text),
                                            ) {
                                                eprintln!("Failed to send message: {}", e);
                                            }
                                        }
                                    }
                                    Tab::Gemini => {
                                        // Gemini AI tab
                                        let timestamp = Local::now().format("%I:%M%p");
                                        let network_name = app.config.network.name.clone();

                                        // Handle commands
                                        if text == "/clear" {
                                            if let Some(ref mut gemini) = app.gemini_chat {
                                                gemini.clear_history();
                                            }
                                            app.ai_buffer.clear();
                                            app.push_ai(format!(
                                                "[{}] *** Conversation cleared ***",
                                                timestamp
                                            ));
                                            app.ai_buffer.scroll_to_bottom();
                                            let _ = app.serial.write_str(&app.ai_buffer.render());
                                        } else if text == "/help" {
                                            app.push_ai(format!(
                                                "[{}] *** /clear, /dos, /unix, /pdp, /apple ***",
                                                timestamp
                                            ));
                                            app.ai_buffer.scroll_to_bottom();
                                            let _ = app.serial.write_str(&app.ai_buffer.render());
                                        } else if text == "/dos"
                                            || text == "/unix"
                                            || text == "/pdp"
                                            || text == "/apple"
                                        {
                                            // Set up simulation mode
                                            let (system_prompt, startup_prompt, mode_name) =
                                                match text.as_str() {
                                                    "/dos" => (
                                                        "You are simulating an MS-DOS 6.22 command prompt on a 386DX-40 PC with 4MB RAM. \
                                                    Respond exactly as MS-DOS would, including the C:\\> prompt. \
                                                    Support common DOS commands like DIR, CD, TYPE, COPY, DEL, MD, RD, VER, MEM, etc. \
                                                    Be authentic to the era. Only output plain text.",
                                                        "Power on the computer and show the boot sequence and DOS prompt.",
                                                        "MS-DOS 6.22",
                                                    ),
                                                    "/unix" => (
                                                        "You are simulating a UNIX System V Release 4 shell on a workstation. \
                                                    Respond exactly as a UNIX shell would, including the $ prompt. \
                                                    Support common UNIX commands like ls, cd, cat, cp, rm, mkdir, rmdir, pwd, who, ps, etc. \
                                                    Be authentic to classic UNIX. Only output plain text.",
                                                        "Show the login prompt, then log in as 'guest' and show the shell prompt.",
                                                        "UNIX System V",
                                                    ),
                                                    "/pdp" => (
                                                        "You are simulating a PDP-11 running RT-11. \
                                                    Respond exactly as RT-11 would, including the . prompt. \
                                                    Support common RT-11 commands like DIR, TYPE, COPY, DELETE, RENAME, etc. \
                                                    Be authentic to the DEC PDP-11 era. Only output plain text.",
                                                        "Power on and show the RT-11 boot sequence and monitor prompt.",
                                                        "PDP-11 RT-11",
                                                    ),
                                                    "/apple" => (
                                                        "You are simulating an Apple II with Applesoft BASIC and ProDOS. \
                                                    Respond exactly as an Apple II would, including the ] prompt for BASIC. \
                                                    Support Applesoft BASIC commands and ProDOS commands like CATALOG, PREFIX, etc. \
                                                    Be authentic to the Apple II era. Only output plain text in uppercase.",
                                                        "Power on and show the Apple II boot sequence with ProDOS and BASIC prompt.",
                                                        "Apple II",
                                                    ),
                                                    _ => unreachable!(),
                                                };

                                            // Set system prompt first (separate borrow)
                                            if let Some(ref mut gemini) = app.gemini_chat {
                                                gemini.set_system_prompt(system_prompt.to_string());
                                            }

                                            app.ai_buffer.clear();
                                            app.ai_buffer.push(format!(
                                                "[{}] *** {} simulation started ***",
                                                timestamp, mode_name
                                            ));
                                            app.ai_buffer.scroll_to_bottom();
                                            let _ = app.serial.write_str(&app.ai_buffer.render());

                                            // Prepare AI response line - show "thinking" while waiting for first token
                                            let ai_prefix =
                                                format!("[{}] ", Local::now().format("%I:%M%p"));

                                            // Show thinking indicator initially
                                            let mut got_first_token = false;
                                            app.ai_buffer
                                                .push(format!("{}<Booting...>", ai_prefix));
                                            let _ = app.serial.write_str(&app.ai_buffer.render());

                                            // Collect the full response for logging
                                            let mut full_response = String::new();

                                            // Stream the startup response
                                            app.ai_processing = true;
                                            if let Some(ref mut gemini) = app.gemini_chat {
                                                let result = gemini.send_message_streaming(startup_prompt, |chunk| {
                                                    full_response.push_str(chunk);
                                                    for ch in chunk.chars() {
                                                        if !got_first_token {
                                                            got_first_token = true;
                                                            app.ai_buffer.update_last_line(&ai_prefix);
                                                        }

                                                        if ch == '\n' {
                                                            app.ai_buffer.push("  ".to_string());
                                                            if app.ai_buffer.is_full() {
                                                                let _ = app.serial.write_str(&app.ai_buffer.render());
                                                            } else {
                                                                let _ = app.serial.write_str(&app.ai_buffer.render_bottom_lines(2));
                                                            }
                                                        } else if !ch.is_control() {
                                                            let wrapped = app.ai_buffer.type_char(ch, "  ");

                                                            if wrapped {
                                                                if app.ai_buffer.is_full() {
                                                                    let _ = app.serial.write_str(&app.ai_buffer.render());
                                                                } else {
                                                                    let _ = app.serial.write_str(&app.ai_buffer.render_bottom_lines(2));
                                                                }
                                                            } else {
                                                                let _ = app.serial.write_str(&app.ai_buffer.render_last_line());
                                                            }

                                                            std::thread::sleep(Duration::from_millis(10));
                                                        }
                                                    }
                                                }).await;

                                                if let Err(e) = result {
                                                    let timestamp = Local::now().format("%I:%M%p");
                                                    app.ai_buffer.push(format!(
                                                        "[{}] *** Error: {} ***",
                                                        timestamp, e
                                                    ));
                                                    app.ai_buffer.scroll_to_bottom();
                                                    let _ = app
                                                        .serial
                                                        .write_str(&app.ai_buffer.render());
                                                }
                                            }
                                            app.ai_processing = false;
                                            let _ = app.serial.clear_input();

                                            // Log the response
                                            if let Some(ref mut logger) = app.logger {
                                                logger.log_ai(&format!(
                                                    "{}{}",
                                                    ai_prefix,
                                                    full_response.replace('\n', " ")
                                                ));
                                            }
                                        } else if let Some(ref mut gemini) = app.gemini_chat {
                                            // Show user message (use client name like in chat tab)
                                            let user_msg = format!(
                                                "[{}] {}: {}",
                                                timestamp, network_name, text
                                            );
                                            if let Some(ref mut logger) = app.logger {
                                                logger.log_ai(&user_msg);
                                            }
                                            app.ai_buffer.push(user_msg);
                                            app.ai_buffer.scroll_to_bottom();
                                            let _ = app.serial.write_str(&app.ai_buffer.render());

                                            // Prepare AI response line - show "thinking" while waiting for first token
                                            let ai_prefix =
                                                format!("[{}] ", Local::now().format("%I:%M%p"));

                                            // Show thinking indicator initially
                                            let mut got_first_token = false;
                                            app.ai_buffer
                                                .push(format!("{}<Thinking...>", ai_prefix));
                                            let _ = app.serial.write_str(&app.ai_buffer.render());

                                            // Collect the full response for logging
                                            let mut full_response = String::new();

                                            // Stream the response - show characters as they arrive
                                            app.ai_processing = true;
                                            let result = gemini
                                                .send_message_streaming(&text, |chunk| {
                                                    full_response.push_str(chunk);
                                                    for ch in chunk.chars() {
                                                        // On first real character, replace thinking with actual content
                                                        if !got_first_token {
                                                            got_first_token = true;
                                                            // Reset the line to just the prefix (removing <Thinking...>)
                                                            app.ai_buffer
                                                                .update_last_line(&ai_prefix);
                                                        }

                                                        if ch == '\n' {
                                                            // Handle newline by starting a new indented line
                                                            app.ai_buffer.push("  ".to_string());
                                                            if app.ai_buffer.is_full() {
                                                                let _ = app.serial.write_str(
                                                                    &app.ai_buffer.render(),
                                                                );
                                                            } else {
                                                                let _ = app.serial.write_str(
                                                                    &app.ai_buffer
                                                                        .render_bottom_lines(2),
                                                                );
                                                            }
                                                        } else if !ch.is_control() {
                                                            let wrapped =
                                                                app.ai_buffer.type_char(ch, "  ");

                                                            if wrapped {
                                                                // If we wrapped, we might have modified the previous line (word wrap)
                                                                // If the buffer is full, we need to redraw everything to show the scroll
                                                                if app.ai_buffer.is_full() {
                                                                    let _ = app.serial.write_str(
                                                                        &app.ai_buffer.render(),
                                                                    );
                                                                } else {
                                                                    // Otherwise just render the last 2 lines
                                                                    let _ = app.serial.write_str(
                                                                        &app.ai_buffer
                                                                            .render_bottom_lines(2),
                                                                    );
                                                                }
                                                            } else {
                                                                // Otherwise just render the current line
                                                                let _ = app.serial.write_str(
                                                                    &app.ai_buffer
                                                                        .render_last_line(),
                                                                );
                                                            }

                                                            // Add a small delay for typing effect
                                                            std::thread::sleep(
                                                                Duration::from_millis(10),
                                                            );
                                                        }
                                                    }
                                                })
                                                .await;
                                            app.ai_processing = false;
                                            let _ = app.serial.clear_input();

                                            // Log the complete AI response
                                            if let Some(ref mut logger) = app.logger {
                                                logger.log_ai(&format!(
                                                    "{}{}",
                                                    ai_prefix,
                                                    full_response.replace('\n', " ")
                                                ));
                                            }

                                            match result {
                                                Ok(_) => {
                                                    // Response is already fully rendered and wrapped by type_char
                                                }
                                                Err(e) => {
                                                    let timestamp = Local::now().format("%I:%M%p");
                                                    app.push_ai(format!(
                                                        "[{}] *** Error: {} ***",
                                                        timestamp, e
                                                    ));
                                                    app.ai_buffer.scroll_to_bottom();
                                                    let _ = app
                                                        .serial
                                                        .write_str(&app.ai_buffer.render());
                                                }
                                            }
                                        }
                                    }
                                    // Call and Tunes are handled before the line buffer check
                                    Tab::Call | Tab::Tunes => unreachable!(),
                                }
                            }
                        }
                        InputEvent::Backspace => {
                            // Backspace
                            if app.ai_processing {
                                continue;
                            }
                            if app.active_tab != Tab::Call
                                && app.active_tab != Tab::Tunes
                                && !app.line_buffer.is_empty()
                                && app.input_cursor > 0
                            {
                                let char_idx = app.input_cursor - 1;
                                let byte_idx = app
                                    .line_buffer
                                    .chars()
                                    .take(char_idx)
                                    .map(|c| c.len_utf8())
                                    .sum();
                                app.line_buffer.remove(byte_idx);
                                app.input_cursor -= 1;
                                // Redraw input line
                                let _ = app.serial.write_str(&redraw_input(
                                    &app.config.network.name,
                                    &app.line_buffer,
                                    app.input_cursor,
                                    width,
                                ));
                            }
                        }
                        InputEvent::CtrlC => {
                            // Ctrl+C - Clear buffer or reset AI
                            match app.active_tab {
                                Tab::Chat => {
                                    app.chat_buffer.clear();
                                    let _ = app.serial.write_str(&app.chat_buffer.render());
                                }
                                Tab::Gemini => {
                                    if let Some(ref mut gemini) = app.gemini_chat {
                                        gemini.clear_history();
                                    }
                                    app.ai_buffer.clear();
                                    let timestamp = Local::now().format("%I:%M%p");
                                    app.push_ai(format!(
                                        "[{}] *** Conversation cleared ***",
                                        timestamp
                                    ));
                                    let _ = app.serial.write_str(&app.ai_buffer.render());
                                }
                                Tab::Call => {
                                    // Do nothing for Call tab
                                }
                                Tab::Tunes => {
                                    // Ctrl+C in Tunes - stop playback
                                    if let Some(ref tunes) = app.tunes_state {
                                        tunes.stop();
                                        let _ = app.serial.write_str(&tunes.render());
                                    }
                                }
                            }
                        }
                        InputEvent::Tab => {
                            // Tab key - switch tabs
                            let prev_tab = app.active_tab;
                            let gemini_available = app.gemini_chat.is_some();
                            let tunes_available = app.tunes_available();
                            app.active_tab = app.active_tab.next(
                                gemini_available,
                                app.active_call.is_some(),
                                tunes_available,
                            );

                            // Reset video state when switching tabs
                            app.last_rendered_frame = None;

                            // Handle webcam state
                            if let Some(cam) = &app.webcam {
                                if app.active_tab == Tab::Call {
                                    cam.start().await;
                                } else if prev_tab == Tab::Call && app.active_call.is_none() {
                                    cam.stop().await;
                                }
                            }

                            // Redraw tab bar and content
                            let _ = app.serial.write_str(&redraw_tab_bar(
                                app.active_tab,
                                gemini_available,
                                tunes_available,
                                app.active_call.as_deref(),
                                width,
                            ));

                            match app.active_tab {
                                Tab::Chat => {
                                    let _ = app.serial.write_str(&init_split_screen_with_tabs(
                                        &app.config.network.name,
                                        app.active_tab,
                                        gemini_available,
                                        tunes_available,
                                        app.active_call.as_deref(),
                                        None,
                                        width,
                                    ));
                                    let _ = app.serial.write_str(&app.chat_buffer.render());
                                    let _ = app.serial.write_str(&redraw_input(
                                        &app.config.network.name,
                                        &app.line_buffer,
                                        app.input_cursor,
                                        width,
                                    ));
                                }
                                Tab::Gemini => {
                                    let _ = app.serial.write_str(&init_split_screen_with_tabs(
                                        &app.config.network.name,
                                        app.active_tab,
                                        gemini_available,
                                        tunes_available,
                                        app.active_call.as_deref(),
                                        None,
                                        width,
                                    ));
                                    let _ = app.serial.write_str(&app.ai_buffer.render());
                                    let _ = app.serial.write_str(&redraw_input(
                                        &app.config.network.name,
                                        &app.line_buffer,
                                        app.input_cursor,
                                        width,
                                    ));
                                }
                                Tab::Call => {
                                    let status = app.active_call.as_ref().map(|peer_name| {
                                        format!(
                                            "Call session with {}. Press Space to hang up.",
                                            peer_name
                                        )
                                    });
                                    let _ = app.serial.write_str(&init_split_screen_with_tabs(
                                        &app.config.network.name,
                                        app.active_tab,
                                        gemini_available,
                                        tunes_available,
                                        app.active_call.as_deref(),
                                        status.as_deref(),
                                        width,
                                    ));
                                }
                                Tab::Tunes => {
                                    let _ = app.serial.write_str(&init_split_screen_with_tabs(
                                        &app.config.network.name,
                                        app.active_tab,
                                        gemini_available,
                                        tunes_available,
                                        app.active_call.as_deref(),
                                        None,
                                        width,
                                    ));
                                    if let Some(ref tunes) = app.tunes_state {
                                        let _ = app.serial.write_str(&tunes.render());
                                    }
                                }
                            }
                        }
                        InputEvent::CtrlR => {
                            // Ctrl+R - Refresh screen (useful if terminal reconnects)
                            let status = if app.active_tab == Tab::Call {
                                app.active_call.as_ref().map(|peer_name| {
                                    format!(
                                        "Call session with {}. Press Space to hang up.",
                                        peer_name
                                    )
                                })
                            } else {
                                None
                            };
                            let gemini_available = app.gemini_chat.is_some();
                            let tunes_available = app.tunes_available();
                            let _ = app.serial.write_str(&init_split_screen_with_tabs(
                                &app.config.network.name,
                                app.active_tab,
                                gemini_available,
                                tunes_available,
                                app.active_call.as_deref(),
                                status.as_deref(),
                                width,
                            ));
                            match app.active_tab {
                                Tab::Chat => {
                                    let _ = app.serial.write_str(&app.chat_buffer.render());
                                    let _ = app.serial.write_str(&redraw_input(
                                        &app.config.network.name,
                                        &app.line_buffer,
                                        app.input_cursor,
                                        width,
                                    ));
                                }
                                Tab::Gemini => {
                                    let _ = app.serial.write_str(&app.ai_buffer.render());
                                    let _ = app.serial.write_str(&redraw_input(
                                        &app.config.network.name,
                                        &app.line_buffer,
                                        app.input_cursor,
                                        width,
                                    ));
                                }
                                Tab::Call => {
                                    // Nothing else to render for Call
                                }
                                Tab::Tunes => {
                                    if let Some(ref tunes) = app.tunes_state {
                                        let _ = app.serial.write_str(&tunes.render());
                                    }
                                }
                            }
                        }
                        InputEvent::Space => {
                            if app.active_tab == Tab::Call {
                                // Space bar in Call tab - Hang up
                                if let Some(peer_name) = app.active_call.take() {
                                    // Send hangup message
                                    if peer_name != app.config.network.name
                                        && let Some(peer) = app
                                            .net_node
                                            .peers()
                                            .iter()
                                            .find(|p| p.name == peer_name)
                                    {
                                        let msg = Message::CallHangup {
                                            from: app.config.network.name.clone(),
                                        };
                                        if let Err(e) = futures::executor::block_on(
                                            app.net_node.send_to(&msg, peer.addr),
                                        ) {
                                            eprintln!("Failed to send hangup: {}", e);
                                        }
                                    }

                                    // Notify local user
                                    let timestamp = Local::now().format("%I:%M%p");
                                    app.push_chat(format!(
                                        "[{}] *** Call with {} ended ***",
                                        timestamp, peer_name
                                    ));

                                    app.last_rendered_frame = None;
                                    app.call_last_packet = None;
                                    app.call_connected = false;
                                    // Stop webcam
                                    if let Some(cam) = &app.webcam {
                                        cam.stop().await;
                                    }
                                    // Switch back to Chat
                                    app.active_tab = Tab::Chat;
                                    let gemini_available = app.gemini_chat.is_some();
                                    let tunes_available = app.tunes_available();
                                    let _ = app.serial.write_str(&init_split_screen_with_tabs(
                                        &app.config.network.name,
                                        app.active_tab,
                                        gemini_available,
                                        tunes_available,
                                        app.active_call.as_deref(),
                                        None,
                                        width,
                                    ));
                                    let _ = app.serial.write_str(&app.chat_buffer.render());
                                    let _ = app.serial.write_str(&redraw_input(
                                        &app.config.network.name,
                                        &app.line_buffer,
                                        app.input_cursor,
                                        width,
                                    ));
                                }
                            } else if app.active_tab == Tab::Tunes {
                                // Space in Tunes - toggle pause/resume, or play if stopped
                                if let Some(ref mut tunes) = app.tunes_state {
                                    if tunes.is_active() {
                                        tunes.toggle_pause();
                                    } else {
                                        // Nothing playing - start playback
                                        if let Err(e) = tunes.play_selected() {
                                            eprintln!("Failed to play: {}", e);
                                        }
                                    }
                                    let _ = app.serial.write_str(&tunes.render());
                                }
                            } else if app.active_tab != Tab::Call {
                                // Space is also a printable character in other tabs
                                if !app.ai_processing && app.line_buffer.len() < max_input_len {
                                    let byte_idx = app
                                        .line_buffer
                                        .chars()
                                        .take(app.input_cursor)
                                        .map(|c| c.len_utf8())
                                        .sum();
                                    app.line_buffer.insert(byte_idx, ' ');
                                    app.input_cursor += 1;
                                    let _ = app.serial.write_str(&redraw_input(
                                        &app.config.network.name,
                                        &app.line_buffer,
                                        app.input_cursor,
                                        width,
                                    ));
                                }
                            }
                        }
                        InputEvent::Char(c) => {
                            if app.active_tab != Tab::Call && app.active_tab != Tab::Tunes {
                                if app.ai_processing {
                                    continue;
                                }
                                // Printable character - only accept if under max length
                                if app.line_buffer.len() < max_input_len {
                                    let byte_idx = app
                                        .line_buffer
                                        .chars()
                                        .take(app.input_cursor)
                                        .map(|c| c.len_utf8())
                                        .sum();
                                    app.line_buffer.insert(byte_idx, c);
                                    app.input_cursor += 1;
                                    // Redraw input area to handle wrapping
                                    let _ = app.serial.write_str(&redraw_input(
                                        &app.config.network.name,
                                        &app.line_buffer,
                                        app.input_cursor,
                                        width,
                                    ));
                                }
                                // Silently ignore input when buffer is full
                            }
                        }
                        InputEvent::Escape(_) | InputEvent::Ignore => {
                            // Handled above or ignored
                        }
                    }
                }
            }
            Err(_e) => {
                if app.running.load(Ordering::SeqCst) {
                    // Serial port disconnected
                    app.serial.mark_disconnected();
                    eprintln!("Serial port disconnected, will attempt to reconnect...");
                }
            }
        }
    }

    // Send leave message to all peers
    eprintln!("\nNotifying peers of departure...");
    let peer_count = app.net_node.peer_count();
    if peer_count > 0 {
        let _ = futures::executor::block_on(async {
            app.net_node
                .broadcast(&Message::Leave {
                    name: app.config.network.name.clone(),
                })
                .await
        });
        // Brief delay to ensure packets are sent before closing socket
        std::thread::sleep(Duration::from_millis(50));
        eprintln!("Notified {} peer(s).", peer_count);
    }

    // Clean up terminal
    eprintln!("Cleaning up terminal...");
    match app.serial.write_str(&cleanup_split_screen(width)) {
        Ok(_) => eprintln!("Terminal cleanup sent."),
        Err(e) => eprintln!("Failed to send terminal cleanup: {}", e),
    }

    // Clean up
    app.net_recv_task.abort();
}
