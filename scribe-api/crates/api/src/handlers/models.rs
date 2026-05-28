use crate::{envelope::ApiResponse, error::ApiError, state::ApiState};
use axum::{
    Json,
    extract::{Query, State},
};
use scribe_config::ModelPriceConfig;
use scribe_domain::ModelKind;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub(crate) struct ModelsQuery {
    #[serde(rename = "type")]
    model_type: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDto {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub model_type: String,
    pub price_unit: String,
    pub credits_per_unit: u64,
}

pub(crate) async fn list_models(
    State(state): State<ApiState>,
    Query(query): Query<ModelsQuery>,
) -> Result<Json<ApiResponse<Vec<ModelDto>>>, ApiError> {
    let kind = query
        .model_type
        .as_deref()
        .map(parse_model_kind)
        .transpose()?;
    let models = state
        .pricing()
        .priced_models(kind)
        .into_iter()
        .map(|(id, model)| to_dto(id, model))
        .collect();
    Ok(Json(ApiResponse::ok(models)))
}

fn parse_model_kind(value: &str) -> Result<ModelKind, ApiError> {
    match value {
        "asr" => Ok(ModelKind::Asr),
        "text" => Ok(ModelKind::Text),
        _ => Err(ApiError::unprocessable("model_type_invalid")),
    }
}

fn to_dto(id: &str, model: &ModelPriceConfig) -> ModelDto {
    ModelDto {
        provider: model.provider.as_str().to_string(),
        id: id.to_string(),
        name: model.display_name.clone(),
        model_type: model.model_type.as_str().to_string(),
        price_unit: model.unit.as_str().to_string(),
        credits_per_unit: model.credits_per_unit,
    }
}
