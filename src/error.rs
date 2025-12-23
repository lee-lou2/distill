use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use thiserror::Error;

use crate::models::ScrapeResponse;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Timeout exceeded: {0}")]
    Timeout(String),

    #[error("Browser error: {0}")]
    Browser(String),

    #[error("Unauthorized: Invalid or missing API key")]
    Unauthorized,

    #[error("Gemini API key not configured")]
    GeminiKeyNotConfigured,

    #[error("LLM provider error: {0}")]
    LlmProvider(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Timeout(_) => "TIMEOUT_EXCEEDED",
            AppError::Browser(_) => "BROWSER_ERROR",
            AppError::Unauthorized => "UNAUTHORIZED",
            AppError::GeminiKeyNotConfigured => "GEMINI_KEY_NOT_CONFIGURED",
            AppError::LlmProvider(_) => "LLM_PROVIDER_ERROR",
            AppError::InvalidRequest(_) => "INVALID_REQUEST",
            AppError::Internal(_) => "INTERNAL_ERROR",
        }
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            AppError::Timeout(_) => StatusCode::REQUEST_TIMEOUT,
            AppError::Browser(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::GeminiKeyNotConfigured => StatusCode::SERVICE_UNAVAILABLE,
            AppError::LlmProvider(_) => StatusCode::BAD_GATEWAY,
            AppError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let response = ScrapeResponse::error(self.code(), &self.to_string());
        (status, Json(response)).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Error Codes ====================

    #[test]
    fn error_code_timeout() {
        assert_eq!(AppError::Timeout("test".to_string()).code(), "TIMEOUT_EXCEEDED");
    }

    #[test]
    fn error_code_browser() {
        assert_eq!(AppError::Browser("test".to_string()).code(), "BROWSER_ERROR");
    }

    #[test]
    fn error_code_unauthorized() {
        assert_eq!(AppError::Unauthorized.code(), "UNAUTHORIZED");
    }

    #[test]
    fn error_code_gemini_not_configured() {
        assert_eq!(AppError::GeminiKeyNotConfigured.code(), "GEMINI_KEY_NOT_CONFIGURED");
    }

    #[test]
    fn error_code_llm_provider() {
        assert_eq!(AppError::LlmProvider("test".to_string()).code(), "LLM_PROVIDER_ERROR");
    }

    #[test]
    fn error_code_invalid_request() {
        assert_eq!(AppError::InvalidRequest("test".to_string()).code(), "INVALID_REQUEST");
    }

    #[test]
    fn error_code_internal() {
        assert_eq!(AppError::Internal("test".to_string()).code(), "INTERNAL_ERROR");
    }

    // ==================== Status Codes ====================

    #[test]
    fn status_code_timeout() {
        assert_eq!(AppError::Timeout("test".to_string()).status_code(), StatusCode::REQUEST_TIMEOUT);
    }

    #[test]
    fn status_code_browser() {
        assert_eq!(AppError::Browser("test".to_string()).status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn status_code_unauthorized() {
        assert_eq!(AppError::Unauthorized.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn status_code_gemini_not_configured() {
        assert_eq!(AppError::GeminiKeyNotConfigured.status_code(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn status_code_llm_provider() {
        assert_eq!(AppError::LlmProvider("test".to_string()).status_code(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn status_code_invalid_request() {
        assert_eq!(AppError::InvalidRequest("test".to_string()).status_code(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn status_code_internal() {
        assert_eq!(AppError::Internal("test".to_string()).status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ==================== Display Messages ====================

    #[test]
    fn display_timeout() {
        let err = AppError::Timeout("10s".to_string());
        assert!(err.to_string().contains("10s"));
    }

    #[test]
    fn display_browser() {
        let err = AppError::Browser("crash".to_string());
        assert!(err.to_string().contains("crash"));
    }

    #[test]
    fn display_unauthorized() {
        let err = AppError::Unauthorized;
        assert!(err.to_string().contains("API key"));
    }

    #[test]
    fn display_invalid_request() {
        let err = AppError::InvalidRequest("bad url".to_string());
        assert!(err.to_string().contains("bad url"));
    }
}
