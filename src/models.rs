use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DEFAULT_MODEL: &str = "gemini-3-flash-preview";

#[derive(Debug, Deserialize)]
pub struct ScrapeRequest {
    pub url: String,
    #[serde(default = "default_output_format")]
    pub output_format: OutputFormat,
    pub analysis_request: Option<AnalysisRequest>,
}

fn default_output_format() -> OutputFormat {
    OutputFormat::Markdown
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Markdown,
    Html,
}

#[derive(Debug, Deserialize)]
pub struct AnalysisRequest {
    #[serde(default = "default_model")]
    pub model: String,
    pub prompt: String,
    pub response_schema: serde_json::Value,
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

#[derive(Debug, Serialize)]
pub struct ScrapeResponse {
    pub success: bool,
    pub data: Option<ScrapeData>,
    pub error: Option<ErrorDetail>,
}

impl ScrapeResponse {
    pub fn success(data: ScrapeData) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(code: &str, message: &str) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(ErrorDetail {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ScrapeData {
    pub metadata: PageMetadata,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_error: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PageMetadata {
    pub title: String,
    pub og_tags: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
}

/// Browser JS extraction result
#[derive(Debug, Deserialize)]
pub struct PageExtractResult {
    pub title: String,
    pub og_tags: HashMap<String, String>,
    pub body_html: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== ScrapeRequest ====================

    #[test]
    fn scrape_request_minimal() {
        let json = r#"{"url": "https://example.com"}"#;
        let req: ScrapeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.url, "https://example.com");
        assert_eq!(req.output_format, OutputFormat::Markdown);
        assert!(req.analysis_request.is_none());
    }

    #[test]
    fn scrape_request_html_format() {
        let json = r#"{"url": "https://example.com", "output_format": "html"}"#;
        let req: ScrapeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.output_format, OutputFormat::Html);
    }

    #[test]
    fn scrape_request_with_analysis() {
        let json = r#"{
            "url": "https://example.com",
            "analysis_request": {
                "prompt": "Summarize",
                "response_schema": {"type": "object"}
            }
        }"#;
        let req: ScrapeRequest = serde_json::from_str(json).unwrap();
        let analysis = req.analysis_request.unwrap();
        assert_eq!(analysis.prompt, "Summarize");
        assert_eq!(analysis.model, DEFAULT_MODEL);
    }

    #[test]
    fn scrape_request_custom_model() {
        let json = r#"{
            "url": "https://example.com",
            "analysis_request": {
                "model": "gemini-pro",
                "prompt": "Summarize",
                "response_schema": {}
            }
        }"#;
        let req: ScrapeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.analysis_request.unwrap().model, "gemini-pro");
    }

    // ==================== ScrapeResponse ====================

    #[test]
    fn scrape_response_success() {
        let data = ScrapeData {
            metadata: PageMetadata {
                title: "Test".to_string(),
                og_tags: HashMap::new(),
            },
            content: "Content".to_string(),
            analysis_result: None,
            analysis_error: None,
        };
        let resp = ScrapeResponse::success(data);
        assert!(resp.success);
        assert!(resp.data.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn scrape_response_error() {
        let resp = ScrapeResponse::error("TEST_ERROR", "Something failed");
        assert!(!resp.success);
        assert!(resp.data.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, "TEST_ERROR");
        assert_eq!(err.message, "Something failed");
    }

    #[test]
    fn scrape_response_json_omits_none() {
        let data = ScrapeData {
            metadata: PageMetadata {
                title: "Test".to_string(),
                og_tags: HashMap::new(),
            },
            content: "Content".to_string(),
            analysis_result: None,
            analysis_error: None,
        };
        let json = serde_json::to_string(&ScrapeResponse::success(data)).unwrap();
        assert!(!json.contains("analysis_result"));
        assert!(!json.contains("analysis_error"));
    }

    #[test]
    fn scrape_response_json_includes_analysis() {
        let data = ScrapeData {
            metadata: PageMetadata {
                title: "Test".to_string(),
                og_tags: HashMap::new(),
            },
            content: "Content".to_string(),
            analysis_result: Some(serde_json::json!({"summary": "test"})),
            analysis_error: None,
        };
        let json = serde_json::to_string(&ScrapeResponse::success(data)).unwrap();
        assert!(json.contains("analysis_result"));
        assert!(json.contains("summary"));
    }

    // ==================== PageExtractResult ====================

    #[test]
    fn page_extract_result_parse() {
        let json = r#"{
            "title": "Test Page",
            "og_tags": {"og:title": "OG Title"},
            "body_html": "<body>Hello</body>"
        }"#;
        let result: PageExtractResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.title, "Test Page");
        assert_eq!(result.og_tags.get("og:title").unwrap(), "OG Title");
        assert_eq!(result.body_html, "<body>Hello</body>");
    }

    #[test]
    fn page_extract_result_empty_og() {
        let json = r#"{
            "title": "",
            "og_tags": {},
            "body_html": "<body></body>"
        }"#;
        let result: PageExtractResult = serde_json::from_str(json).unwrap();
        assert!(result.og_tags.is_empty());
    }
}
