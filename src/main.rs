mod app;
mod config;
mod dec_graphics;
mod gemini;
mod network;
mod serial;
mod terminal;
mod webcam;

use app::App;
use chrono::Local;
use clap::Parser;
use config::Config;
use network::{Message, PEER_TIMEOUT, PeerEvent};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use terminal::{cleanup_split_screen, init_split_screen_with_tabs, max_input_length, redraw_input, redraw_tab_bar, render_stream, Tab, generate_waiting_for_peer_frame};

#[derive(Parser, Debug)]
#[command(name = "wormhole")]
#[command(about = "A serial terminal chat application for VT120 terminals")]
struct Args {
    /// Path to the configuration file
    #[arg(short, long, default_value = "wormhole.ini")]
    config: PathBuf,
}

#[tokio::main]
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
    println!("  UPnP: {}", if config.network.upnp { "enabled" } else { "disabled" });
    if config.network.peers.is_empty() {
        println!("  Peers: (none configured)");
    } else {
        println!("  Peers: {}", config.network.peers);
    }
    println!();
    println!("Webcam:");
    if let Some(ref device) = config.webcam.device {
        println!("  Device: {}", device);
        if config.webcam.fps > 0 {
            println!("  FPS: {}", config.webcam.fps);
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

    // Main loop - handle serial I/O and network messages
    let max_input_len = max_input_length(&app.config.network.name);
    let mut serial_buf = [0u8; 256];
    let mut escape_buf: Vec<u8> = Vec::new(); // Buffer for escape sequences
    let mut last_reconnect_attempt = std::time::Instant::now();
    const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);
    
    // Calculate frame delay based on baud rate to avoid flooding the serial link
    // Frame size ~ 65x20 chars + overhead ~ 1500 bytes. 
    let bytes_per_frame = 2000;
    let chars_per_sec = std::cmp::max(app.config.serial.baud_rate / 10, 1);
    let calculated_fps = (chars_per_sec as f64 / bytes_per_frame as f64).max(0.5).min(30.0);
    
    let target_fps = if app.config.webcam.fps > 0 {
        app.config.webcam.fps as f64
    } else {
        calculated_fps
    };
    
    let frame_delay = Duration::from_secs_f64(1.0 / target_fps);
    let mut last_frame_time = std::time::Instant::now().checked_sub(frame_delay).unwrap_or_else(std::time::Instant::now);

    while app.running.load(Ordering::SeqCst) {
        // Handle serial reconnection if disconnected
        if !app.serial.is_connected() {
            if last_reconnect_attempt.elapsed() >= RECONNECT_INTERVAL {
                last_reconnect_attempt = std::time::Instant::now();
                eprintln!("Attempting to reconnect to {}...", app.serial.port_path());
                match app.serial.reconnect() {
                    Ok(()) => {
                        eprintln!("Reconnected to serial port!");
                        // Reinitialize the terminal UI
                        let call_status = if let Some(peer_name) = &app.active_call {
                            Some(format!("Call session with {}. Press Space to hang up.", peer_name))
                        } else {
                            None
                        };
                        let gemini_available = app.gemini_chat.is_some();
                        let _ = app.serial.write_str(&init_split_screen_with_tabs(&app.config.network.name, app.active_tab, gemini_available, app.active_call.as_deref(), call_status.as_deref()));
                        // Render the active buffer
                        match app.active_tab {
                            Tab::Chat => {
                                let _ = app.serial.write_str(&app.chat_buffer.render());
                                let _ = app.serial.write_str(&redraw_input(&app.config.network.name, &app.line_buffer));
                            }
                            Tab::Gemini => {
                                let _ = app.serial.write_str(&app.ai_buffer.render());
                                let _ = app.serial.write_str(&redraw_input(&app.config.network.name, &app.line_buffer));
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
                            app.chat_buffer.push(format!("[{}] {} shared an image:", timestamp, from));
                            for line in text.strip_prefix("[IMAGE]\n").unwrap_or(&text).lines() {
                                app.chat_buffer.push(line.to_string());
                            }
                        } else {
                            let formatted = format!("[{}] {}: {}", timestamp, from, text);
                            app.chat_buffer.push(formatted);
                        }
                        app.chat_buffer.scroll_to_bottom();
                    }
                    Message::StreamFrame { from, lines } => {
                        app.current_stream_frame = Some((from, lines));
                    }
                    _ => {}
                }
            }
            while let Ok(event) = app.peer_event_rx.try_recv() {
                let timestamp = Local::now().format("%I:%M%p");
                let msg = match event {
                    PeerEvent::Joined { name } => format!("[{}] *** {} has joined ***", timestamp, name),
                    PeerEvent::Left { name } => format!("[{}] *** {} has left ***", timestamp, name),
                };
                app.chat_buffer.push(msg);
            }
            continue;
        }

        // Prune stale peers periodically (allows reconnection after timeout)
        let timed_out_peers = app.net_node.prune_peers(PEER_TIMEOUT);
        for peer in timed_out_peers {
            let timestamp = Local::now().format("%I:%M%p");
            let msg = format!("[{}] *** {} has timed out ***", timestamp, peer.name);
            app.chat_buffer.push(msg);
            if app.active_tab == Tab::Chat {
                let _ = app.serial.write_str(&app.chat_buffer.render());
            }
        }

        // Check for peer events (join/leave)
        while let Ok(event) = app.peer_event_rx.try_recv() {
            let timestamp = Local::now().format("%I:%M%p");
            let msg = match event {
                PeerEvent::Joined { name } => format!("[{}] *** {} has joined ***", timestamp, name),
                PeerEvent::Left { name } => format!("[{}] *** {} has left ***", timestamp, name),
            };
            app.chat_buffer.push(msg);
            if app.active_tab == Tab::Chat {
                let _ = app.serial.write_str(&app.chat_buffer.render());
            }
        }

        // Check for discovered peers
        while let Ok(peer) = app.discovery_rx.try_recv() {
            // Skip if we already know this peer and they haven't timed out
            if app.net_node.has_peer(peer.addr, PEER_TIMEOUT) {
                // Update last_seen for active peers
                app.net_node.touch_peer(peer.addr);
                continue;
            }
            eprintln!("Discovered peer: {} at {}", peer.name, peer.addr);
            // Add peer and send join message (notification shown when we receive their Join reply)
            if let Err(e) = app.net_node.connect_to_peer(peer.addr).await {
                eprintln!("Failed to connect to discovered peer: {}", e);
            } else {
                app.net_node.add_peer(peer.name.clone(), peer.addr);
            }
        }

        // Check for incoming network messages - drain all pending
        let mut had_messages = false;
        while let Ok(msg) = app.net_rx.try_recv() {
            match msg {
                Message::Chat { from, text } => {
                    let timestamp = Local::now().format("%I:%M%p");
                    
                    // Check if this is an image message
                    if text.starts_with("[IMAGE]\n") {
                        // Add header
                        app.chat_buffer.push(format!("[{}] {} shared an image:", timestamp, from));
                        // Add each line of the ASCII art
                        for line in text.strip_prefix("[IMAGE]\n").unwrap_or(&text).lines() {
                            app.chat_buffer.push(line.to_string());
                        }
                    } else if text.starts_with("\x01ACTION ") {
                        // IRC-style /me action
                        let action = text.strip_prefix("\x01ACTION ").unwrap_or("");
                        let formatted = format!("[{}] * {} {}", timestamp, from, action);
                        app.chat_buffer.push(formatted);
                    } else {
                        // Regular chat message
                        let formatted = format!("[{}] {}: {}", timestamp, from, text);
                        app.chat_buffer.push(formatted);
                    }
                    app.chat_buffer.scroll_to_bottom();
                    had_messages = true;
                }
                Message::StreamFrame { from, lines } => {
                    app.current_stream_frame = Some((from, lines));
                }
                _ => {}
            }
        }
        // Render once after processing all messages
        if had_messages && app.active_tab == Tab::Chat {
            if let Err(e) = app.serial.write_str(&app.chat_buffer.render()) {
                eprintln!("Serial write error: {}", e);
                break;
            }
        }

        // Handle Call tab logic
        if app.active_tab == Tab::Call && last_frame_time.elapsed() >= frame_delay {
            last_frame_time = std::time::Instant::now();
            let mut frame_to_render = None;
            let mut sender_name = String::new();

            // Try to capture from webcam if available
            let mut local_frame = None;
            if let Some(cam) = &mut app.webcam {
                match cam.capture_frame() {
                    Ok(lines) => {
                        // Broadcast frame
                        let msg = Message::StreamFrame {
                            from: app.config.network.name.clone(),
                            lines: lines.clone(),
                        };
                        
                        // Broadcast (blocking for now, but UDP is fast)
                        if let Err(e) = futures::executor::block_on(app.net_node.broadcast(&msg)) {
                            eprintln!("Failed to broadcast stream frame: {}", e);
                        }
                        
                        local_frame = Some(lines);
                    }
                    Err(e) => {
                        eprintln!("Webcam capture error: {}", e);
                    }
                }
            }

            // Determine what to render
            // 1. If we are calling someone, try to show their video
            if let Some(peer_name) = &app.active_call {
                if let Some((from, lines)) = &app.current_stream_frame {
                    if from == peer_name {
                        frame_to_render = Some(lines.clone());
                        sender_name = from.clone();
                    }
                }
                
                // 2. If we haven't found their video yet, and we are calling "yourself", show local video
                if frame_to_render.is_none() && peer_name == &app.config.network.name {
                    if let Some(lines) = local_frame.clone() {
                        frame_to_render = Some(lines);
                        sender_name = app.config.network.name.clone();
                    }
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
                 if let Some((from, lines)) = &app.current_stream_frame {
                    frame_to_render = Some(lines.clone());
                    sender_name = from.clone();
                } else if app.active_call.is_none() {
                    // Only show mirror if not in a call
                    if let Some(lines) = local_frame {
                        frame_to_render = Some(lines);
                        sender_name = app.config.network.name.clone();
                    }
                }
            }

            // Render if we have a frame
            if let Some(lines) = frame_to_render {
                if let Err(e) = app.serial.write_str(&render_stream(&sender_name, &lines)) {
                    eprintln!("Serial write error in Call tab: {}", e);
                }
            }
        }

        // Check for serial input
        match app.serial.read(&mut serial_buf) {
            Ok(0) => {
                // No data available - yield briefly to allow async tasks to run
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Ok(n) => {
                // Process input character by character
                for &byte in &serial_buf[..n] {
                    // Handle escape sequences for Page Up/Down
                    if !escape_buf.is_empty() {
                        escape_buf.push(byte);
                        
                        // Check for complete escape sequences
                        // Page Up: ESC [ 5 ~ or ESC [ 5 ; ...
                        // Page Down: ESC [ 6 ~ or ESC [ 6 ; ...
                        if escape_buf.len() >= 3 {
                            let seq = &escape_buf[..];
                            if seq == b"\x1b[5~" {
                                // Page Up - scroll up (on active buffer)
                                let active_buffer = if app.active_tab == Tab::Chat { &mut app.chat_buffer } else { &mut app.ai_buffer };
                                active_buffer.scroll_up(10);
                                let _ = app.serial.write_str(&active_buffer.render());
                                escape_buf.clear();
                                continue;
                            } else if seq == b"\x1b[6~" {
                                // Page Down - scroll down (on active buffer)
                                let active_buffer = if app.active_tab == Tab::Chat { &mut app.chat_buffer } else { &mut app.ai_buffer };
                                active_buffer.scroll_down(10);
                                let _ = app.serial.write_str(&active_buffer.render());
                                escape_buf.clear();
                                continue;
                            } else if seq.len() > 6 || (seq.len() >= 3 && seq[seq.len()-1] == b'~') {
                                // Unknown or complete sequence, discard
                                escape_buf.clear();
                                continue;
                            }
                            // Still building sequence, continue
                            continue;
                        }
                        continue;
                    }
                    
                    match byte {
                        0x1b => {
                            // Start of escape sequence
                            escape_buf.push(byte);
                        }
                        b'\r' | b'\n' => {
                            if !app.line_buffer.is_empty() {
                                let text = app.line_buffer.clone();
                                app.line_buffer.clear();
                                
                                // Redraw empty input line first
                                if app.active_tab != Tab::Call {
                                    let _ = app.serial.write_str(&redraw_input(&app.config.network.name, ""));
                                }

                                // Handle input based on active tab
                                match app.active_tab {
                                    Tab::Call => {
                                        // No input handling for Call tab
                                    }
                                    Tab::Chat => {
                                        // P2P Chat tab - handle commands and messages
                                        if text.starts_with('/') {
                                            if text.starts_with("/me ") {
                                                let action = text.strip_prefix("/me ").unwrap_or("");
                                                let timestamp = Local::now().format("%I:%M%p");
                                                let formatted = format!("[{}] * {} {}", timestamp, app.config.network.name, action);
                                                app.chat_buffer.push(formatted);
                                                app.chat_buffer.scroll_to_bottom();
                                                let _ = app.serial.write_str(&app.chat_buffer.render());
                                                
                                                // Broadcast to peers
                                                let action_msg = format!("\x01ACTION {}", action);
                                                if let Err(e) = futures::executor::block_on(app.net_node.send_chat(&action_msg)) {
                                                    eprintln!("Failed to send action: {}", e);
                                                }
                                            } else {
                                                match text.as_str() {
                                                    "/image" => {
                                                        // Capture webcam snapshot
                                                        let timestamp = Local::now().format("%I:%M%p");
                                                        match webcam::capture_ascii_snapshot(app.config.webcam.device.as_deref()) {
                                                            Ok(lines) => {
                                                                // Add header
                                                                app.chat_buffer.push(format!("[{}] {} shared an image:", timestamp, app.config.network.name));
                                                                // Add each line of the ASCII art
                                                                for line in &lines {
                                                                    app.chat_buffer.push(line.clone());
                                                                }
                                                                app.chat_buffer.scroll_to_bottom();
                                                                let _ = app.serial.write_str(&app.chat_buffer.render());
                                                                
                                                                // Also send to peers as multi-line message
                                                                let img_msg = format!("[IMAGE]\n{}", lines.join("\n"));
                                                                if let Err(e) = futures::executor::block_on(app.net_node.send_chat(&img_msg)) {
                                                                    eprintln!("Failed to send image: {}", e);
                                                                }
                                                            }
                                                            Err(e) => {
                                                                let err_msg = format!("[{}] *** Webcam error: {} ***", timestamp, e);
                                                                app.chat_buffer.push(err_msg);
                                                                app.chat_buffer.scroll_to_bottom();
                                                                let _ = app.serial.write_str(&app.chat_buffer.render());
                                                            }
                                                        }
                                                    }
                                                    "/help" => {
                                                        let timestamp = Local::now().format("%I:%M%p");
                                                        app.chat_buffer.push(format!("[{}] *** /clear, /who, /image, /me <action>, /call <peer> ***", timestamp));
                                                        app.chat_buffer.scroll_to_bottom();
                                                        let _ = app.serial.write_str(&app.chat_buffer.render());
                                                    }
                                                    "/clear" => {
                                                        app.chat_buffer.clear();
                                                        let _ = app.serial.write_str(&app.chat_buffer.render());
                                                    }
                                                    "/who" => {
                                                        let timestamp = Local::now().format("%I:%M%p");
                                                        let peers = app.net_node.peers();
                                                        if peers.is_empty() {
                                                            app.chat_buffer.push(format!("[{}] *** No peers connected ***", timestamp));
                                                        } else {
                                                            app.chat_buffer.push(format!("[{}] *** Connected Peers ({}) ***", timestamp, peers.len()));
                                                            for peer in peers {
                                                                app.chat_buffer.push(format!("  - {} ({})", peer.name, peer.addr));
                                                            }
                                                        }
                                                        app.chat_buffer.scroll_to_bottom();
                                                        let _ = app.serial.write_str(&app.chat_buffer.render());
                                                    }
                                                    _ => {
                                                        if text.starts_with("/call ") {
                                                            let peer_name = text.strip_prefix("/call ").unwrap_or("").trim();
                                                            if !peer_name.is_empty() {
                                                                app.active_call = Some(peer_name.to_string());
                                                                app.active_tab = Tab::Call;
                                                                
                                                                // Start webcam
                                                                if let Some(cam) = &mut app.webcam {
                                                                    let _ = cam.start();
                                                                }

                                                                // Redraw UI
                                                                let status = format!("Call session with {}. Press Space to hang up.", peer_name);
                                                                let gemini_available = app.gemini_chat.is_some();
                                                                let _ = app.serial.write_str(&init_split_screen_with_tabs(&app.config.network.name, app.active_tab, gemini_available, app.active_call.as_deref(), Some(&status)));
                                                            }
                                                        } else {
                                                            let timestamp = Local::now().format("%I:%M%p");
                                                            app.chat_buffer.push(format!("[{}] *** Unknown command: {} ***", timestamp, text));
                                                            app.chat_buffer.scroll_to_bottom();
                                                            let _ = app.serial.write_str(&app.chat_buffer.render());
                                                        }
                                                    }
                                                }
                                            }
                                        } else {
                                            // Regular chat message
                                            let timestamp = Local::now().format("%I:%M%p");
                                            let our_msg = format!("[{}] {}: {}", timestamp, app.config.network.name, text);
                                            app.chat_buffer.push(our_msg);
                                            app.chat_buffer.scroll_to_bottom();
                                            let _ = app.serial.write_str(&app.chat_buffer.render());

                                            // Broadcast to peers
                                            if let Err(e) = futures::executor::block_on(app.net_node.send_chat(&text)) {
                                                eprintln!("Failed to send message: {}", e);
                                            }
                                        }
                                    }
                                    Tab::Gemini => {
                                        // Gemini AI tab
                                        if let Some(ref mut gemini) = app.gemini_chat {
                                            let timestamp = Local::now().format("%I:%M%p");
                                            
                                            // Handle commands
                                            if text == "/clear" {
                                                gemini.clear_history();
                                                app.ai_buffer.clear();
                                                app.ai_buffer.push(format!("[{}] *** Conversation cleared ***", timestamp));
                                                app.ai_buffer.scroll_to_bottom();
                                                let _ = app.serial.write_str(&app.ai_buffer.render());
                                            } else if text == "/help" {
                                                app.ai_buffer.push(format!("[{}] *** /clear ***", timestamp));
                                                app.ai_buffer.scroll_to_bottom();
                                                let _ = app.serial.write_str(&app.ai_buffer.render());
                                            } else {
                                                // Show user message (use client name like in chat tab)
                                                app.ai_buffer.push(format!("[{}] {}: {}", timestamp, app.config.network.name, text));
                                                app.ai_buffer.scroll_to_bottom();
                                                let _ = app.serial.write_str(&app.ai_buffer.render());
                                                
                                                // Prepare AI response line - show "thinking" while waiting for first token
                                                let ai_prefix = format!("[{}] AI: ", Local::now().format("%I:%M%p"));
                                                
                                                // Show thinking indicator initially
                                                let mut display_line = format!("{}<Thinking...>", ai_prefix);
                                                let mut got_first_token = false;
                                                app.ai_buffer.push(display_line.clone());
                                                let _ = app.serial.write_str(&app.ai_buffer.render());
                                                
                                                let max_display = 76usize;
                                                
                                                // Stream the response - show characters as they arrive
                                                let result = gemini.send_message_streaming(&text, |chunk| {
                                                    for ch in chunk.chars() {
                                                        // On first real character, replace thinking with actual content
                                                        if !got_first_token {
                                                            got_first_token = true;
                                                            display_line = ai_prefix.clone();
                                                        }
                                                        
                                                        if ch == '\n' {
                                                            // Don't show newlines in streaming display
                                                            display_line.push(' ');
                                                        } else if !ch.is_control() {
                                                            display_line.push(ch);
                                                        }
                                                        
                                                        // Truncate display line if too long (show latest chars)
                                                        if display_line.len() > max_display {
                                                            // Keep prefix and show "..." plus recent text
                                                            let suffix_len = max_display - ai_prefix.len() - 3;
                                                            let suffix_start = display_line.len() - suffix_len;
                                                            display_line = format!("{}...{}", ai_prefix, &display_line[suffix_start..]);
                                                        }
                                                        
                                                        app.ai_buffer.update_last_line(&display_line);
                                                        let _ = app.serial.write_str(&app.ai_buffer.render());
                                                    }
                                                }).await;
                                                
                                                match result {
                                                    Ok(full_response) => {
                                                        // Now format the complete response properly with word wrapping
                                                        // Remove the streaming line first
                                                        app.ai_buffer.pop_last();
                                                        
                                                        // Word-wrap the full response
                                                        let max_len = 76usize;
                                                        let indent = "  ";
                                                        
                                                        // Process the response, collapsing multiple newlines
                                                        let normalized: String = full_response
                                                            .lines()
                                                            .map(|l| l.trim())
                                                            .filter(|l| !l.is_empty())
                                                            .collect::<Vec<_>>()
                                                            .join(" ");
                                                        
                                                        let words: Vec<&str> = normalized.split_whitespace().collect();
                                                        let mut current_line = ai_prefix.clone();
                                                        
                                                        for word in words {
                                                            let space_needed = if current_line == ai_prefix || current_line == indent { 0 } else { 1 };
                                                            
                                                            if current_line.len() + space_needed + word.len() > max_len {
                                                                // Line is full, push it and start new line
                                                                app.ai_buffer.push(current_line);
                                                                current_line = format!("{}{}", indent, word);
                                                            } else {
                                                                // Add word to current line
                                                                if space_needed > 0 {
                                                                    current_line.push(' ');
                                                                }
                                                                current_line.push_str(word);
                                                            }
                                                        }
                                                        
                                                        // Push remaining line
                                                        if !current_line.is_empty() && current_line != indent {
                                                            app.ai_buffer.push(current_line);
                                                        }
                                                        
                                                        app.ai_buffer.scroll_to_bottom();
                                                        let _ = app.serial.write_str(&app.ai_buffer.render());
                                                    }
                                                    Err(e) => {
                                                        app.ai_buffer.pop_last();
                                                        let timestamp = Local::now().format("%I:%M%p");
                                                        app.ai_buffer.push(format!("[{}] *** Error: {} ***", timestamp, e));
                                                        app.ai_buffer.scroll_to_bottom();
                                                        let _ = app.serial.write_str(&app.ai_buffer.render());
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        0x7f | 0x08 => {
                            // Backspace
                            if app.active_tab != Tab::Call && !app.line_buffer.is_empty() {
                                app.line_buffer.pop();
                                // Redraw input line
                                let _ = app.serial.write_str(&redraw_input(&app.config.network.name, &app.line_buffer));
                            }
                        }
                        0x03 => {
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
                                    app.ai_buffer.push(format!("[{}] *** Conversation cleared ***", timestamp));
                                    let _ = app.serial.write_str(&app.ai_buffer.render());
                                }
                                Tab::Call => {
                                    // Do nothing for Call tab
                                }
                            }
                        }
                        0x09 => {
                            // Tab key - switch tabs
                            let prev_tab = app.active_tab;
                            let gemini_available = app.gemini_chat.is_some();
                            app.active_tab = app.active_tab.next(gemini_available, app.active_call.is_some());
                            
                            // Handle webcam state
                            if let Some(cam) = &mut app.webcam {
                                if app.active_tab == Tab::Call {
                                    let _ = cam.start();
                                } else if prev_tab == Tab::Call {
                                    let _ = cam.stop();
                                }
                            }

                            // Redraw tab bar and content
                            let _ = app.serial.write_str(&redraw_tab_bar(app.active_tab, gemini_available, app.active_call.as_deref()));
                            
                            match app.active_tab {
                                Tab::Chat => {
                                    let _ = app.serial.write_str(&init_split_screen_with_tabs(&app.config.network.name, app.active_tab, gemini_available, app.active_call.as_deref(), None));
                                    let _ = app.serial.write_str(&app.chat_buffer.render());
                                    let _ = app.serial.write_str(&redraw_input(&app.config.network.name, &app.line_buffer));
                                }
                                Tab::Gemini => {
                                    let _ = app.serial.write_str(&init_split_screen_with_tabs(&app.config.network.name, app.active_tab, gemini_available, app.active_call.as_deref(), None));
                                    let _ = app.serial.write_str(&app.ai_buffer.render());
                                    let _ = app.serial.write_str(&redraw_input(&app.config.network.name, &app.line_buffer));
                                }
                                Tab::Call => {
                                    let status = if let Some(peer_name) = &app.active_call {
                                        Some(format!("Call session with {}. Press Space to hang up.", peer_name))
                                    } else {
                                        None
                                    };
                                    let _ = app.serial.write_str(&init_split_screen_with_tabs(&app.config.network.name, app.active_tab, gemini_available, app.active_call.as_deref(), status.as_deref()));
                                }
                            }
                        }
                        0x12 => {
                            // Ctrl+R - Refresh screen (useful if terminal reconnects)
                            let status = if app.active_tab == Tab::Call {
                                if let Some(peer_name) = &app.active_call {
                                    Some(format!("Call session with {}. Press Space to hang up.", peer_name))
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            let gemini_available = app.gemini_chat.is_some();
                            let _ = app.serial.write_str(&init_split_screen_with_tabs(&app.config.network.name, app.active_tab, gemini_available, app.active_call.as_deref(), status.as_deref()));
                            match app.active_tab {
                                Tab::Chat => {
                                    let _ = app.serial.write_str(&app.chat_buffer.render());
                                    let _ = app.serial.write_str(&redraw_input(&app.config.network.name, &app.line_buffer));
                                }
                                Tab::Gemini => {
                                    let _ = app.serial.write_str(&app.ai_buffer.render());
                                    let _ = app.serial.write_str(&redraw_input(&app.config.network.name, &app.line_buffer));
                                }
                                Tab::Call => {
                                    // Nothing else to render for Call
                                }
                            }
                        }
                        _ => {
                            if app.active_tab == Tab::Call && byte == 0x20 {
                                // Space bar in Call tab - Hang up
                                if app.active_call.is_some() {
                                    app.active_call = None;
                                    // Stop webcam
                                    if let Some(cam) = &mut app.webcam {
                                        let _ = cam.stop();
                                    }
                                    // Switch back to Chat
                                    app.active_tab = Tab::Chat;
                                    let gemini_available = app.gemini_chat.is_some();
                                    let _ = app.serial.write_str(&init_split_screen_with_tabs(&app.config.network.name, app.active_tab, gemini_available, app.active_call.as_deref(), None));
                                    let _ = app.serial.write_str(&app.chat_buffer.render());
                                    let _ = app.serial.write_str(&redraw_input(&app.config.network.name, &app.line_buffer));
                                }
                            } else if app.active_tab != Tab::Call && byte >= 0x20 && byte < 0x7f {
                                // Printable character - only accept if under max length
                                if app.line_buffer.len() < max_input_len {
                                    app.line_buffer.push(byte as char);
                                    // Redraw input area to handle wrapping
                                    let _ = app.serial.write_str(&redraw_input(&app.config.network.name, &app.line_buffer));
                                }
                                // Silently ignore input when buffer is full
                            }
                        }
                    }
                }
            }
            Err(e) => {
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
            app.net_node.broadcast(&Message::Leave {
                name: app.config.network.name.clone(),
            }).await
        });
        // Brief delay to ensure packets are sent before closing socket
        std::thread::sleep(Duration::from_millis(50));
        eprintln!("Notified {} peer(s).", peer_count);
    }

    // Clean up terminal
    eprintln!("Cleaning up terminal...");
    match app.serial.write_str(&cleanup_split_screen()) {
        Ok(_) => eprintln!("Terminal cleanup sent."),
        Err(e) => eprintln!("Failed to send terminal cleanup: {}", e),
    }

    // Clean up
    app.net_recv_task.abort();
}
