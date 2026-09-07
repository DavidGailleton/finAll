//! The asset-to-asset conversion primitive.
//!
//! [`convert`] is pure decimal arithmetic. The caller supplies every input a
//! currency conversion needs — the amount, the exchange rate, the source and
//! target asset ids, the valuation timestamp, the target asset's minor-unit
//! precision and the rounding mode. Nothing is read from the database and no
//! rate is looked up or inferred, so a conversion never happens implicitly.
//!
//! `rate` is the price of one unit of the source asset expressed in the target
//! asset (target units per source unit); the converted amount is
//! `amount * rate` rounded to `target_minor_units` fractional digits with
//! `rounding_mode`. A caller holding a rate quoted the other way round must
//! invert it before calling.

use bigdecimal::{BigDecimal, RoundingMode, Signed};
use sqlx::types::chrono::{DateTime, Utc};
use sqlx::types::Uuid;

/// Why a conversion could not be performed.
///
/// The primitive does no I/O, so the only failure is a caller-supplied input
/// that the domain rejects; the message is safe to show a user.
#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("{0}")]
    InvalidInput(&'static str),
}

/// A completed conversion together with the inputs that determined it, so the
/// caller can persist or display the amount without losing its provenance.
pub struct Conversion {
    /// `amount * rate`, rounded to `minor_units` fractional digits with
    /// `rounding_mode`.
    pub converted_amount: BigDecimal,
    pub source_asset_id: Uuid,
    pub target_asset_id: Uuid,
    /// Target units per source unit, as supplied by the caller.
    pub rate: BigDecimal,
    pub valuation_timestamp: DateTime<Utc>,
    /// Fractional digits the result was rounded to.
    pub minor_units: i16,
    pub rounding_mode: RoundingMode,
}

/// Convert `amount`, denominated in the source asset, into the target asset.
///
/// See the module docs for the meaning of `rate`. A zero or negative `amount`
/// is valid and its sign is preserved.
///
/// Returns [`ConversionError::InvalidInput`] when `source_asset_id` equals
/// `target_asset_id`, when `rate` is not strictly positive (mirroring the
/// `asset_rates_rate_positive` check), or when `target_minor_units` is outside
/// the `0..=18` range the schema allows for a fiat asset.
pub fn convert(
    amount: &BigDecimal,
    rate: BigDecimal,
    source_asset_id: Uuid,
    target_asset_id: Uuid,
    valuation_timestamp: DateTime<Utc>,
    target_minor_units: i16,
    rounding_mode: RoundingMode,
) -> Result<Conversion, ConversionError> {
    if source_asset_id == target_asset_id {
        return Err(ConversionError::InvalidInput(
            "source and target assets must be different",
        ));
    }
    if !rate.is_positive() {
        return Err(ConversionError::InvalidInput(
            "exchange rate must be greater than zero",
        ));
    }
    if !(0..=18).contains(&target_minor_units) {
        return Err(ConversionError::InvalidInput(
            "target minor units must be between 0 and 18",
        ));
    }

    let converted_amount =
        (amount * &rate).with_scale_round(i64::from(target_minor_units), rounding_mode);

    Ok(Conversion {
        converted_amount,
        source_asset_id,
        target_asset_id,
        rate,
        valuation_timestamp,
        minor_units: target_minor_units,
        rounding_mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(s: &str) -> BigDecimal {
        BigDecimal::from_str(s).expect("valid decimal literal")
    }

    fn ts() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("valid fixed timestamp")
    }

    fn source() -> Uuid {
        Uuid::from_u128(1)
    }

    fn target() -> Uuid {
        Uuid::from_u128(2)
    }

    #[test]
    fn multiplies_amount_by_rate_at_target_precision() {
        let c = convert(
            &dec("100"),
            dec("1.25"),
            source(),
            target(),
            ts(),
            2,
            RoundingMode::HalfEven,
        )
        .expect("conversion succeeds");

        assert_eq!(c.converted_amount, dec("125.00"));
        assert_eq!(c.rate, dec("1.25"));
        assert_eq!(c.minor_units, 2);
    }

    #[test]
    fn preserves_negative_amount() {
        let c = convert(
            &dec("-50"),
            dec("2"),
            source(),
            target(),
            ts(),
            2,
            RoundingMode::HalfEven,
        )
        .expect("conversion succeeds");

        assert_eq!(c.converted_amount, dec("-100.00"));
    }

    #[test]
    fn zero_amount_is_not_missing_data() {
        let c = convert(
            &dec("0"),
            dec("1.25"),
            source(),
            target(),
            ts(),
            2,
            RoundingMode::HalfEven,
        )
        .expect("conversion succeeds");

        assert_eq!(c.converted_amount, dec("0.00"));
    }

    #[test]
    fn applies_the_requested_rounding_mode_at_the_half() {
        let half_even = convert(
            &dec("1"),
            dec("1.005"),
            source(),
            target(),
            ts(),
            2,
            RoundingMode::HalfEven,
        )
        .expect("conversion succeeds");
        assert_eq!(half_even.converted_amount, dec("1.00"));

        let half_up = convert(
            &dec("1"),
            dec("1.005"),
            source(),
            target(),
            ts(),
            2,
            RoundingMode::HalfUp,
        )
        .expect("conversion succeeds");
        assert_eq!(half_up.converted_amount, dec("1.01"));
    }

    #[test]
    fn rejects_non_positive_rate() {
        for rate in ["0", "-1.5"] {
            let result = convert(
                &dec("100"),
                dec(rate),
                source(),
                target(),
                ts(),
                2,
                RoundingMode::HalfEven,
            );
            assert!(matches!(result, Err(ConversionError::InvalidInput(_))));
        }
    }

    #[test]
    fn rejects_equal_source_and_target() {
        let result = convert(
            &dec("100"),
            dec("1.25"),
            source(),
            source(),
            ts(),
            2,
            RoundingMode::HalfEven,
        );
        assert!(matches!(result, Err(ConversionError::InvalidInput(_))));
    }

    #[test]
    fn rejects_minor_units_outside_schema_range() {
        for minor_units in [-1, 19] {
            let result = convert(
                &dec("100"),
                dec("1.25"),
                source(),
                target(),
                ts(),
                minor_units,
                RoundingMode::HalfEven,
            );
            assert!(matches!(result, Err(ConversionError::InvalidInput(_))));
        }
    }
}
