use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::net::SocketAddr;
use std::io::Write;
use tokio::sync::mpsc;
use chrono::Local;

use crate::config::Config;
use crate::serial::Serial;
use crate::network::{self, NetworkNode, Discovery, DiscoveredPeer, Message, PeerEvent, run_discovery};
use crate::webcam::WebcamStream;
use crate::gemini::GeminiChat;
use crate::terminal::{ChatBuffer, Tab, init_split_screen_with_tabs};

/// Helper macro to print status and flush stdout
macro_rules! status {
    ($($arg:tt)*) => {{
        print!($($arg)*);
        let _ = std::io::stdout().flush();
    }};
}

pub struct App {
    pub config: Config,
    pub serial: Serial,
    pub net_node: NetworkNode,
    pub webcam: Option<WebcamStream>,
    pub gemini_chat: Option<GeminiChat>,
    pub chat_buffer: ChatBuffer,
    pub ai_buffer: ChatBuffer,
    pub active_tab: Tab,
    pub active_call: Option<String>,
    pub current_stream_frame: Option<(String, Vec<String>)>,
    pub line_buffer: String,
    pub running: Arc<AtomicBool>,
    
    // Channels
    pub discovery_rx: mpsc::Receiver<DiscoveredPeer>,
    pub net_rx: mpsc::Receiver<Message>,
    pub peer_event_rx: mpsc::Receiver<PeerEvent>,
    
    // Task handles
    pub net_recv_task: tokio::task::JoinHandle<()>,
    // We don't strictly need to keep the discovery task handle if it handles its own shutdown or we don't await it
}

impl App {
    pub async fn new(config: Config, running: Arc<AtomicBool>) -> Result<Self, Box<dyn std::error::Error>> {
        // Open serial port
        status!("Opening serial port {}... ", config.serial.port);
        let mut serial = match Serial::open(&config.serial) {
            Ok(s) => {
                println!("OK");
                s
            }
            Err(e) => {
                println!("FAILED");
                eprintln!("Error: {}", e);
                return Err(e.into());
            }
        };

        // Set up networking
        status!("Starting network on port {}... ", config.network.port);
        let mut net_node = match NetworkNode::new(config.network.name.clone(), config.network.port).await
        {
            Ok(n) => {
                println!("OK");
                n
            }
            Err(e) => {
                println!("FAILED");
                eprintln!("Network error: {}", e);
                // Explicitly drop serial before exiting to release the port
                drop(serial);
                eprintln!("Serial port released.");
                return Err(e.into());
            }
        };

        // Try STUN discovery
        status!("Discovering public endpoint via STUN... ");
        match network::discover_public_endpoint(config.network.port) {
            Ok(addr) => {
                println!("{}", addr);
                net_node.set_public_addr(addr);
            }
            Err(e) => {
                println!("FAILED");
                eprintln!("  {}", e);
            }
        }

        // Try UPnP port forwarding if enabled
        if config.network.upnp {
            status!("Setting up UPnP port forwarding... ");
            match network::setup_port_forward(
                config.network.port,
                config.network.port,
                "Wormhole Chat",
                config.network.bind_ip.as_deref(),
            )
            {
                Ok(addr) => {
                    println!("OK (external port {})", addr);
                }
                Err(e) => {
                    println!("FAILED");
                    eprintln!("  {}", e);
                }
            }
        }

        // Connect to configured peers
        if !config.network.peers.is_empty() {
            println!("Connecting to peers...");
            for peer_str in config.network.peers.split(',') {
                let peer_str = peer_str.trim();
                if let Ok(addr) = peer_str.parse::<SocketAddr>() {
                    status!("  {}... ", addr);
                    match net_node.connect_to_peer(addr).await {
                        Ok(_) => println!("OK"),
                        Err(e) => {
                            println!("FAILED");
                            eprintln!("    {}", e);
                        }
                    }
                } else {
                    println!("  {}... INVALID ADDRESS", peer_str);
                }
            }
        }

        // Set up peer discovery
        status!("Starting LAN discovery... ");
        let discovery = match Discovery::new(config.network.name.clone(), config.network.port).await {
            Ok(d) => {
                println!("OK");
                Arc::new(d)
            }
            Err(e) => {
                println!("FAILED");
                eprintln!("  {} (continuing without LAN discovery)", e);
                // Continue without discovery - we can still connect to manual peers
                Arc::new(Discovery::new(config.network.name.clone(), 0).await.unwrap())
            }
        };

        // Channels for discovered peers
        let (discovery_tx, discovery_rx) = mpsc::channel::<DiscoveredPeer>(32);

        // Shutdown signal for discovery
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Spawn discovery task
        let discovery_clone = Arc::clone(&discovery);
        tokio::spawn(async move {
            run_discovery(discovery_clone, discovery_tx, shutdown_rx).await;
        });

        println!();
        println!("Ready.");
        println!();

        // Create channels for communication between tasks
        let (net_tx, net_rx) = mpsc::channel::<Message>(32);
        let (peer_event_tx, peer_event_rx) = mpsc::channel::<PeerEvent>(32);

        let socket = net_node.socket();
        let running_net = running.clone();

        // Spawn network receive task
        let net_recv_task = tokio::spawn(async move {
            let mut buf = [0u8; 65535]; // Increased buffer size for stream frames
            while running_net.load(Ordering::SeqCst) {
                // Use a short timeout to allow checking the running flag
                tokio::select! {
                    result = socket.recv_from(&mut buf) => {
                        match result {
                            Ok((len, _addr)) => {
                                if let Some(msg) = Message::from_bytes(&buf[..len]) {
                                    match msg {
                                        Message::Chat { .. } => {
                                            let _ = net_tx.send(msg).await;
                                        }
                                        Message::StreamFrame { .. } => {
                                            let _ = net_tx.send(msg).await;
                                        }
                                        Message::Join { name } => {
                                            let _ = peer_event_tx.send(PeerEvent::Joined { name }).await;
                                        }
                                        Message::Leave { name } => {
                                            let _ = peer_event_tx.send(PeerEvent::Left { name }).await;
                                        }
                                        Message::Ping { seq } => {
                                            // Respond with pong
                                            let pong = Message::Pong { seq };
                                            let _ = socket.send_to(&pong.to_bytes(), _addr).await;
                                        }
                                        Message::Pong { .. } => {
                                            // Latency measurement could go here
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Network receive error: {}", e);
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {
                        // Brief check for shutdown - recv_from will wake immediately on data
                    }
                }
            }
        });

        // Initialize Gemini chat if configured
        let gemini_available = GeminiChat::is_available(&config.gemini);
        let gemini_chat = if gemini_available {
            match GeminiChat::new(&config.gemini) {
                Ok(chat) => Some(chat),
                Err(e) => {
                    eprintln!("Warning: Failed to initialize Gemini: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Tab state
        let active_tab = Tab::Chat;
        let active_call: Option<String> = None;

        // Initialize split-screen terminal UI with tabs
        let _ = serial.write_str(&init_split_screen_with_tabs(&config.network.name, active_tab, gemini_available, active_call.as_deref(), None));

        // Create chat buffers for each tab
        let chat_buffer = ChatBuffer::new();
        let webcam = match WebcamStream::new(config.webcam.device.as_deref()) {
            Ok(cam) => {
                eprintln!("Webcam initialized successfully.");
                Some(cam)
            },
            Err(e) => {
                eprintln!("Warning: Failed to initialize webcam: {}", e);
                None
            }
        };

        let mut ai_buffer = ChatBuffer::new();

        // Add welcome message to AI buffer if available
        if gemini_available {
            let timestamp = Local::now().format("%I:%M%p");
            ai_buffer.push(format!("[{}] *** Gemini AI ready. Type to chat! ***", timestamp));
        }

        Ok(Self {
            config,
            serial,
            net_node,
            webcam,
            gemini_chat,
            chat_buffer,
            ai_buffer,
            active_tab,
            active_call,
            current_stream_frame: None,
            line_buffer: String::new(),
            running,
            discovery_rx,
            net_rx,
            peer_event_rx,
            net_recv_task,
        })
    }
}
