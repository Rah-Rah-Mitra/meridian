//! RFC 7807 problem responses (SPEC §10). Detail strings must never carry query
//! text, URLs, client IPs, or secrets — the planner/fetch error types are
//! already scrubbed; keep it that way.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub struct Problem {
    pub status: StatusCode,
    pub title: &'static str,
    pub detail: String,
}

impl Problem {
    pub fn new(status: StatusCode, title: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status,
            title,
            detail: detail.into(),
        }
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "type": "about:blank",
            "title": self.title,
            "status": self.status.as_u16(),
            "detail": self.detail,
        });
        let mut response = (self.status, Json(body)).into_response();
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

pub fn plan_error(e: meridian_query::planner::PlanError) -> Problem {
    use meridian_query::planner::PlanError;
    match &e {
        PlanError::Lane(_) => Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "lane unavailable",
            e.to_string(),
        ),
        PlanError::AnonBusy => Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "anon lane busy",
            e.to_string(),
        ),
        PlanError::Index(_) => Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "index error",
            "search failed",
        ),
        PlanError::Internal(_) => Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal error",
            "search failed",
        ),
    }
}

pub fn fetch_error(e: &meridian_query::FetchError) -> Problem {
    use meridian_query::FetchError;
    let (status, title) = match e {
        FetchError::BadUrl | FetchError::BadRedirect => (StatusCode::BAD_REQUEST, "invalid url"),
        FetchError::Ssrf(_) => (StatusCode::UNPROCESSABLE_ENTITY, "refused by ssrf guard"),
        FetchError::RobotsDenied => (StatusCode::UNPROCESSABLE_ENTITY, "disallowed by robots.txt"),
        FetchError::RobotsUnavailable => (StatusCode::BAD_GATEWAY, "robots.txt unavailable"),
        FetchError::TooLarge => (StatusCode::UNPROCESSABLE_ENTITY, "response too large"),
        FetchError::UnsupportedMediaType => {
            (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported media type")
        }
        FetchError::TooManyRedirects => (StatusCode::BAD_GATEWAY, "too many redirects"),
        FetchError::Http(_) | FetchError::Network(_) => (StatusCode::BAD_GATEWAY, "upstream error"),
        FetchError::Lane(_) => (StatusCode::SERVICE_UNAVAILABLE, "lane unavailable"),
        FetchError::Extract(_) => (StatusCode::UNPROCESSABLE_ENTITY, "extraction failed"),
    };
    Problem::new(status, title, e.to_string())
}
