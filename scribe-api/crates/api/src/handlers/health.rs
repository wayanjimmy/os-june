use axum::http::StatusCode;

pub(crate) async fn livez() -> StatusCode {
    StatusCode::OK
}
