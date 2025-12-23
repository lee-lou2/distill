use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{debug, error, warn};

use crate::error::{AppError, AppResult};
use crate::models::AnalysisRequest;

const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const LLM_TIMEOUT_SECS: u64 = 60;

pub struct GeminiClient {
    http_client: Client,
    api_key: Option<String>,
}

impl GeminiClient {
    pub fn new() -> Self {
        let api_key = std::env::var("GEMINI_API_KEY").ok();

        if api_key.is_none() {
            warn!("GEMINI_API_KEY not set");
        }

        let http_client = Client::builder()
            .timeout(Duration::from_secs(LLM_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");

        Self { http_client, api_key }
    }

    pub fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    pub async fn analyze(&self, content: &str, request: &AnalysisRequest) -> AppResult<Value> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or(AppError::GeminiKeyNotConfigured)?;

        let endpoint = format!(
            "{}/{}:generateContent?key={}",
            GEMINI_API_BASE, request.model, api_key
        );

        let payload = self.build_payload(content, request);

        debug!(model = %request.model, content_len = content.len(), "Calling Gemini API");

        let response = self
            .http_client
            .post(&endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::LlmProvider(format!("Request failed: {}", e)))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| AppError::LlmProvider(format!("Response read failed: {}", e)))?;

        if !status.is_success() {
            error!(status = %status, "Gemini API error");
            return Err(AppError::LlmProvider(format!("Status {}: {}", status, body)));
        }

        let gemini_response: GeminiResponse = serde_json::from_str(&body)
            .map_err(|e| AppError::LlmProvider(format!("Parse failed: {}", e)))?;

        self.extract_output(gemini_response)
    }

    fn build_payload(&self, content: &str, request: &AnalysisRequest) -> Value {
        json!({
            "contents": [{
                "parts": [{
                    "text": format!("{}\n\n---\n\nContent to analyze:\n\n{}", request.prompt, content)
                }]
            }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": request.response_schema
            }
        })
    }

    fn extract_output(&self, response: GeminiResponse) -> AppResult<Value> {
        let text = response
            .candidates
            .into_iter()
            .next()
            .ok_or_else(|| AppError::LlmProvider("No candidates".to_string()))?
            .content
            .parts
            .into_iter()
            .next()
            .ok_or_else(|| AppError::LlmProvider("No parts".to_string()))?
            .text;

        serde_json::from_str(&text)
            .map_err(|e| AppError::LlmProvider(format!("JSON parse failed: {}", e)))
    }
}

impl Default for GeminiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: Content,
}

#[derive(Debug, Deserialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Debug, Deserialize)]
struct Part {
    text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AnalysisRequest;

    fn make_request(prompt: &str, model: &str) -> AnalysisRequest {
        AnalysisRequest {
            model: model.to_string(),
            prompt: prompt.to_string(),
            response_schema: serde_json::json!({"type": "object"}),
        }
    }

    // ==================== build_payload ====================

    #[test]
    fn payload_contains_content() {
        let client = GeminiClient::new();
        let request = make_request("Summarize", "gemini-pro");
        let payload = client.build_payload("Hello world", &request);

        let text = payload["contents"][0]["parts"][0]["text"].as_str().unwrap();
        assert!(text.contains("Hello world"));
    }

    #[test]
    fn payload_contains_prompt() {
        let client = GeminiClient::new();
        let request = make_request("Extract entities", "gemini-pro");
        let payload = client.build_payload("Some text", &request);

        let text = payload["contents"][0]["parts"][0]["text"].as_str().unwrap();
        assert!(text.contains("Extract entities"));
    }

    #[test]
    fn payload_has_json_response_type() {
        let client = GeminiClient::new();
        let request = make_request("Test", "gemini-pro");
        let payload = client.build_payload("Content", &request);

        let mime_type = payload["generationConfig"]["responseMimeType"].as_str().unwrap();
        assert_eq!(mime_type, "application/json");
    }

    #[test]
    fn payload_includes_response_schema() {
        let client = GeminiClient::new();
        let request = AnalysisRequest {
            model: "gemini-pro".to_string(),
            prompt: "Test".to_string(),
            response_schema: serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        };
        let payload = client.build_payload("Content", &request);

        let schema = &payload["generationConfig"]["responseSchema"];
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["name"].is_object());
    }

    // ==================== extract_output ====================

    #[test]
    fn extract_output_valid_json() {
        let client = GeminiClient::new();
        let response = GeminiResponse {
            candidates: vec![Candidate {
                content: Content {
                    parts: vec![Part {
                        text: r#"{"result": "success"}"#.to_string(),
                    }],
                },
            }],
        };

        let output = client.extract_output(response).unwrap();
        assert_eq!(output["result"], "success");
    }

    #[test]
    fn extract_output_no_candidates() {
        let client = GeminiClient::new();
        let response = GeminiResponse { candidates: vec![] };

        let err = client.extract_output(response).unwrap_err();
        assert!(err.to_string().contains("No candidates"));
    }

    #[test]
    fn extract_output_no_parts() {
        let client = GeminiClient::new();
        let response = GeminiResponse {
            candidates: vec![Candidate {
                content: Content { parts: vec![] },
            }],
        };

        let err = client.extract_output(response).unwrap_err();
        assert!(err.to_string().contains("No parts"));
    }

    #[test]
    fn extract_output_invalid_json() {
        let client = GeminiClient::new();
        let response = GeminiResponse {
            candidates: vec![Candidate {
                content: Content {
                    parts: vec![Part {
                        text: "not valid json".to_string(),
                    }],
                },
            }],
        };

        let err = client.extract_output(response).unwrap_err();
        assert!(err.to_string().contains("JSON parse failed"));
    }

    // ==================== is_configured ====================

    #[test]
    fn is_configured_depends_on_env() {
        let client = GeminiClient::new();
        let expected = std::env::var("GEMINI_API_KEY").is_ok();
        assert_eq!(client.is_configured(), expected);
    }
}
