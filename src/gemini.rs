use crate::config::GeminiConfig;
use futures::TryStreamExt;
use gemini_rust::{Gemini, Model};

/// Error type for Gemini operations
#[derive(Debug)]
pub enum GeminiError {
    /// No API key configured
    NoApiKey,
    /// Client creation failed
    ClientError(String),
    /// API request failed
    RequestError(String),
}

impl std::fmt::Display for GeminiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GeminiError::NoApiKey => write!(f, "No Gemini API key configured"),
            GeminiError::ClientError(e) => write!(f, "Gemini client error: {}", e),
            GeminiError::RequestError(e) => write!(f, "Gemini request error: {}", e),
        }
    }
}

impl std::error::Error for GeminiError {}

/// A message in the conversation history
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
}

/// Gemini chat session with conversation history
pub struct GeminiChat {
    client: Gemini,
    system_prompt: Option<String>,
    history: Vec<ChatMessage>,
}

impl GeminiChat {
    /// Create a new Gemini chat session from config
    /// `terminal_width` is the number of columns available for output
    /// `terminal_mode` is the terminal type (e.g., "vt100" or "vt220")
    pub fn new(config: &GeminiConfig, terminal_width: usize, terminal_mode: &str) -> Result<Self, GeminiError> {
        let api_key = config.api_key.as_ref().ok_or(GeminiError::NoApiKey)?;

        // Parse model string to Model enum and create client with that model
        let model: Model = match config.model.as_str() {
            "gemini-2.5-pro" => Model::Gemini25Pro,
            "gemini-2.5-flash-lite" => Model::Gemini25FlashLite,
            "gemini-2.5-flash" => Model::Gemini25Flash,
            custom => Model::Custom(format!("models/{}", custom)),
        };

        let client = Gemini::with_model(api_key, model)
            .map_err(|e| GeminiError::ClientError(e.to_string()))?;

        // Build system prompt with terminal information
        // Account for chat buffer margins (4 chars: left border + padding + right border)
        let available_cols = terminal_width.saturating_sub(4);
        let system_prompt = config.system_prompt.as_ref().map(|prompt| {
            format!(
                "{} The user is on a {} terminal with {} columns. Keep your responses under {} characters per line to avoid wrapping.",
                prompt, terminal_mode.to_uppercase(), terminal_width, available_cols
            )
        });

        Ok(Self {
            client,
            system_prompt,
            history: Vec::new(),
        })
    }

    /// Check if Gemini is configured and available
    pub fn is_available(config: &GeminiConfig) -> bool {
        config.api_key.is_some()
    }

    /// Send a message and stream the response, calling the callback for each chunk
    pub async fn send_message_streaming<F>(
        &mut self,
        message: &str,
        mut on_chunk: F,
    ) -> Result<String, GeminiError>
    where
        F: FnMut(&str),
    {
        // Add user message to history
        self.history.push(ChatMessage {
            role: MessageRole::User,
            content: message.to_string(),
        });

        // Build the request with conversation history
        let mut request = self.client.generate_content();

        // Add system prompt if configured
        if let Some(ref system_prompt) = self.system_prompt {
            request = request.with_system_prompt(system_prompt);
        }

        // Add conversation history
        for msg in &self.history {
            match msg.role {
                MessageRole::User => {
                    request = request.with_user_message(&msg.content);
                }
                MessageRole::Assistant => {
                    request = request.with_model_message(&msg.content);
                }
            }
        }

        // Execute streaming request
        let mut stream = request
            .execute_stream()
            .await
            .map_err(|e| GeminiError::RequestError(e.to_string()))?;

        // Collect the full response while streaming chunks
        let mut full_response = String::new();
        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(|e| GeminiError::RequestError(e.to_string()))?
        {
            let text = chunk.text();
            full_response.push_str(&text);
            on_chunk(&text);
        }

        // Add assistant response to history
        self.history.push(ChatMessage {
            role: MessageRole::Assistant,
            content: full_response.clone(),
        });

        Ok(full_response)
    }

    /// Clear conversation history
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Set a new system prompt (clears history as well)
    pub fn set_system_prompt(&mut self, prompt: String) {
        self.system_prompt = Some(prompt);
        self.history.clear();
    }
}
