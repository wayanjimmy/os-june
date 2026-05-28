use scribe_config::{ModelPriceConfig, ModelType, PriceUnit};
use scribe_domain::{Credits, ModelKind};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct PricingTable {
    models: BTreeMap<String, ModelPriceConfig>,
}

impl PricingTable {
    pub fn new(models: BTreeMap<String, ModelPriceConfig>) -> Self {
        Self { models }
    }

    pub fn price_credits(&self, model_id: &str, units: u64) -> Result<Credits, PricingError> {
        let model = self.models.get(model_id).ok_or(PricingError::NotPriced)?;
        Self::price_model_units(model, units)
    }

    pub fn price_audio_seconds(
        &self,
        model_id: &str,
        seconds: u64,
    ) -> Result<Credits, PricingError> {
        let model = self.models.get(model_id).ok_or(PricingError::NotPriced)?;
        if model.unit != PriceUnit::Seconds {
            return Err(PricingError::WrongUnit);
        }
        Self::price_model_units(model, seconds)
    }

    pub fn price_tokens(&self, model_id: &str, tokens: u64) -> Result<Credits, PricingError> {
        let model = self.models.get(model_id).ok_or(PricingError::NotPriced)?;
        if model.unit != PriceUnit::Tokens {
            return Err(PricingError::WrongUnit);
        }
        Self::price_model_units(model, tokens)
    }

    pub fn has_model(&self, model_id: &str) -> bool {
        self.models.contains_key(model_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &ModelPriceConfig)> {
        self.models.iter()
    }

    pub fn priced_models(&self, kind: Option<ModelKind>) -> Vec<(&String, &ModelPriceConfig)> {
        self.models
            .iter()
            .filter(|(_, model)| {
                kind.is_none_or(|kind| model_type_matches_kind(model.model_type, kind))
            })
            .collect()
    }

    fn price_model_units(model: &ModelPriceConfig, units: u64) -> Result<Credits, PricingError> {
        let credits = units
            .checked_mul(model.credits_per_unit)
            .ok_or(PricingError::Overflow)?;
        Ok(Credits(credits))
    }
}

fn model_type_matches_kind(model_type: ModelType, kind: ModelKind) -> bool {
    matches!(
        (model_type, kind),
        (ModelType::Asr, ModelKind::Asr) | (ModelType::Text, ModelKind::Text)
    )
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum PricingError {
    #[error("model_not_priced")]
    NotPriced,
    #[error("price_unit_mismatch")]
    WrongUnit,
    #[error("price_overflow")]
    Overflow,
}

#[cfg(test)]
mod tests {
    use super::{PricingError, PricingTable};
    use pretty_assertions::assert_eq;
    use scribe_config::{ModelPriceConfig, ModelProvider, ModelType, PriceUnit};
    use std::collections::BTreeMap;

    fn models<const N: usize>(
        values: [(&str, PriceUnit, u64, ModelType); N],
    ) -> BTreeMap<String, ModelPriceConfig> {
        values
            .into_iter()
            .map(|(id, unit, credits_per_unit, model_type)| {
                (
                    id.to_string(),
                    ModelPriceConfig {
                        unit,
                        credits_per_unit,
                        provider: ModelProvider::Openai,
                        model_type,
                        display_name: id.to_string(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn prices_known_models() {
        let table = PricingTable::new(models([
            ("priced-asr", PriceUnit::Seconds, 3, ModelType::Asr),
            ("priced-text", PriceUnit::Tokens, 4, ModelType::Text),
        ]));

        assert_eq!(
            table
                .price_credits("priced-asr", 9)
                .map(|credits| credits.0),
            Ok(27)
        );
        assert_eq!(
            table
                .price_tokens("priced-text", 10)
                .map(|credits| credits.0),
            Ok(40)
        );
    }

    #[test]
    fn rejects_unknown_models() {
        let table = PricingTable::new(BTreeMap::new());
        assert_eq!(
            table.price_credits("missing", 1),
            Err(PricingError::NotPriced)
        );
    }

    #[test]
    fn catches_price_overflow() {
        let table = PricingTable::new(models([(
            "too-large",
            PriceUnit::Tokens,
            u64::MAX,
            ModelType::Text,
        )]));
        assert_eq!(
            table.price_tokens("too-large", 2),
            Err(PricingError::Overflow)
        );
    }

    #[test]
    fn rejects_wrong_unit_for_audio_seconds() {
        let table = PricingTable::new(models([(
            "text-model",
            PriceUnit::Tokens,
            1,
            ModelType::Text,
        )]));
        assert_eq!(
            table.price_audio_seconds("text-model", 1),
            Err(PricingError::WrongUnit)
        );
    }

    #[test]
    fn rejects_wrong_unit_for_tokens() {
        let table = PricingTable::new(models([(
            "asr-model",
            PriceUnit::Seconds,
            1,
            ModelType::Asr,
        )]));
        assert_eq!(
            table.price_tokens("asr-model", 1),
            Err(PricingError::WrongUnit)
        );
    }

    #[test]
    fn has_model_reports_known_ids() {
        let table = PricingTable::new(models([("known", PriceUnit::Seconds, 1, ModelType::Asr)]));
        assert!(table.has_model("known"));
        assert!(!table.has_model("unknown"));
    }
}
