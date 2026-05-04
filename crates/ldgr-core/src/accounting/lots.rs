//! Investment lot tracking and cost basis computation.
//!
//! Pure computation — no I/O. Handles FIFO, LIFO, Average Cost,
//! and Specific Identification methods for lot disposal.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// An investment lot (tax lot).
#[derive(Debug, Clone)]
pub struct Lot {
    pub id: String,
    pub account_id: String,
    pub commodity: String,
    pub quantity: Decimal,
    pub cost_basis: Decimal,
    pub cost_per_unit: Decimal,
    pub cost_commodity: String,
    pub acquisition_date: String,
    pub disposal_date: Option<String>,
    pub disposal_proceeds: Option<Decimal>,
    pub realized_gain: Option<Decimal>,
}

/// Cost basis method for lot disposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostBasisMethod {
    Fifo,
    Lifo,
    AverageCost,
    SpecificId,
}

impl CostBasisMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fifo => "FIFO",
            Self::Lifo => "LIFO",
            Self::AverageCost => "Average",
            Self::SpecificId => "SpecID",
        }
    }
}

/// Holding period classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldingTerm {
    ShortTerm,
    LongTerm,
}

/// Result of disposing shares from lots.
#[derive(Debug, Clone)]
pub struct DisposalResult {
    pub lots_consumed: Vec<LotConsumption>,
    pub total_proceeds: Decimal,
    pub total_cost_basis: Decimal,
    pub realized_gain: Decimal,
}

/// How much of a specific lot was consumed in a disposal.
#[derive(Debug, Clone)]
pub struct LotConsumption {
    pub lot_id: String,
    pub quantity: Decimal,
    pub cost_basis: Decimal,
    pub proceeds: Decimal,
    pub gain: Decimal,
    pub term: HoldingTerm,
}

/// A gain/loss entry for reporting.
#[derive(Debug, Clone)]
pub struct GainEntry {
    pub commodity: String,
    pub quantity: Decimal,
    pub cost_basis: Decimal,
    pub proceeds: Decimal,
    pub gain: Decimal,
    pub term: HoldingTerm,
    pub acquisition_date: String,
    pub disposal_date: String,
}

/// Default holding period threshold in days (1 year).
const LONG_TERM_DAYS: i64 = 365;

/// Determine holding term based on acquisition and disposal dates.
pub fn classify_term(acquisition_date: &str, disposal_date: &str) -> HoldingTerm {
    let Ok(acq) = chrono::NaiveDate::parse_from_str(acquisition_date, "%Y-%m-%d") else {
        return HoldingTerm::ShortTerm;
    };
    let Ok(disp) = chrono::NaiveDate::parse_from_str(disposal_date, "%Y-%m-%d") else {
        return HoldingTerm::ShortTerm;
    };
    if (disp - acq).num_days() > LONG_TERM_DAYS {
        HoldingTerm::LongTerm
    } else {
        HoldingTerm::ShortTerm
    }
}

/// Compute disposal using FIFO (First In, First Out).
///
/// Disposes shares from the oldest lots first.
pub fn dispose_fifo(
    lots: &[Lot],
    quantity: Decimal,
    price_per_unit: Decimal,
    disposal_date: &str,
) -> DisposalResult {
    let mut sorted: Vec<&Lot> = lots.iter().filter(|l| l.disposal_date.is_none()).collect();
    sorted.sort_by(|a, b| a.acquisition_date.cmp(&b.acquisition_date));
    dispose_from_sorted(&sorted, quantity, price_per_unit, disposal_date)
}

/// Compute disposal using LIFO (Last In, First Out).
///
/// Disposes shares from the newest lots first.
pub fn dispose_lifo(
    lots: &[Lot],
    quantity: Decimal,
    price_per_unit: Decimal,
    disposal_date: &str,
) -> DisposalResult {
    let mut sorted: Vec<&Lot> = lots.iter().filter(|l| l.disposal_date.is_none()).collect();
    sorted.sort_by(|a, b| b.acquisition_date.cmp(&a.acquisition_date));
    dispose_from_sorted(&sorted, quantity, price_per_unit, disposal_date)
}

/// Compute disposal using Average Cost.
///
/// Uses the weighted average cost per unit across all open lots.
pub fn dispose_average(
    lots: &[Lot],
    quantity: Decimal,
    price_per_unit: Decimal,
    disposal_date: &str,
) -> DisposalResult {
    let open_lots: Vec<&Lot> = lots.iter().filter(|l| l.disposal_date.is_none()).collect();

    let total_qty: Decimal = open_lots.iter().map(|l| l.quantity).sum();
    let total_cost: Decimal = open_lots.iter().map(|l| l.cost_basis).sum();

    if total_qty.is_zero() || quantity.is_zero() {
        return empty_result();
    }

    let avg_cost_per_unit = total_cost / total_qty;
    let cost_basis = avg_cost_per_unit * quantity;
    let proceeds = price_per_unit * quantity;
    let gain = proceeds - cost_basis;

    // For average cost, we conceptually consume from the oldest lot first
    let term = open_lots.first().map_or(HoldingTerm::ShortTerm, |l| {
        classify_term(&l.acquisition_date, disposal_date)
    });

    DisposalResult {
        lots_consumed: vec![LotConsumption {
            lot_id: "average".into(),
            quantity,
            cost_basis,
            proceeds,
            gain,
            term,
        }],
        total_proceeds: proceeds,
        total_cost_basis: cost_basis,
        realized_gain: gain,
    }
}

/// Compute disposal using Specific Identification.
///
/// Disposes from the specified lot only.
pub fn dispose_specific(
    lot: &Lot,
    quantity: Decimal,
    price_per_unit: Decimal,
    disposal_date: &str,
) -> DisposalResult {
    let cost_basis = lot.cost_per_unit * quantity;
    let proceeds = price_per_unit * quantity;
    let gain = proceeds - cost_basis;
    let term = classify_term(&lot.acquisition_date, disposal_date);

    DisposalResult {
        lots_consumed: vec![LotConsumption {
            lot_id: lot.id.clone(),
            quantity,
            cost_basis,
            proceeds,
            gain,
            term,
        }],
        total_proceeds: proceeds,
        total_cost_basis: cost_basis,
        realized_gain: gain,
    }
}

/// Compute unrealized gain/loss for open lots at a given price.
pub fn unrealized_gain(lots: &[Lot], current_price: Decimal) -> Decimal {
    lots.iter()
        .filter(|l| l.disposal_date.is_none())
        .map(|l| (current_price * l.quantity) - l.cost_basis)
        .sum()
}

// ── Internal ───────────────────────────────────────────────────────────────────

fn dispose_from_sorted(
    sorted_lots: &[&Lot],
    mut remaining: Decimal,
    price_per_unit: Decimal,
    disposal_date: &str,
) -> DisposalResult {
    let mut consumptions = Vec::new();
    let mut total_proceeds = Decimal::ZERO;
    let mut total_cost = Decimal::ZERO;

    for lot in sorted_lots {
        if remaining.is_zero() {
            break;
        }
        let take = remaining.min(lot.quantity);
        let cost = lot.cost_per_unit * take;
        let proceeds = price_per_unit * take;
        let gain = proceeds - cost;
        let term = classify_term(&lot.acquisition_date, disposal_date);

        consumptions.push(LotConsumption {
            lot_id: lot.id.clone(),
            quantity: take,
            cost_basis: cost,
            proceeds,
            gain,
            term,
        });

        total_proceeds += proceeds;
        total_cost += cost;
        remaining -= take;
    }

    DisposalResult {
        realized_gain: total_proceeds - total_cost,
        lots_consumed: consumptions,
        total_proceeds,
        total_cost_basis: total_cost,
    }
}

fn empty_result() -> DisposalResult {
    DisposalResult {
        lots_consumed: Vec::new(),
        total_proceeds: Decimal::ZERO,
        total_cost_basis: Decimal::ZERO,
        realized_gain: Decimal::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lot(id: &str, qty: &str, cpu: &str, date: &str) -> Lot {
        let q: Decimal = qty.parse().unwrap();
        let c: Decimal = cpu.parse().unwrap();
        Lot {
            id: id.into(),
            account_id: "acct".into(),
            commodity: "AAPL".into(),
            quantity: q,
            cost_basis: q * c,
            cost_per_unit: c,
            cost_commodity: "USD".into(),
            acquisition_date: date.into(),
            disposal_date: None,
            disposal_proceeds: None,
            realized_gain: None,
        }
    }

    fn sample_lots() -> Vec<Lot> {
        vec![
            make_lot("L1", "10", "100", "2023-01-15"),
            make_lot("L2", "20", "150", "2023-06-15"),
            make_lot("L3", "15", "200", "2024-01-15"),
        ]
    }

    #[test]
    fn fifo_disposes_oldest_first() {
        let lots = sample_lots();
        let result = dispose_fifo(
            &lots,
            Decimal::new(15, 0),
            Decimal::new(250, 0),
            "2024-06-01",
        );

        assert_eq!(result.lots_consumed.len(), 2);
        assert_eq!(result.lots_consumed[0].lot_id, "L1");
        assert_eq!(result.lots_consumed[0].quantity, Decimal::new(10, 0));
        assert_eq!(result.lots_consumed[1].lot_id, "L2");
        assert_eq!(result.lots_consumed[1].quantity, Decimal::new(5, 0));
    }

    #[test]
    fn lifo_disposes_newest_first() {
        let lots = sample_lots();
        let result = dispose_lifo(
            &lots,
            Decimal::new(15, 0),
            Decimal::new(250, 0),
            "2024-06-01",
        );

        assert_eq!(result.lots_consumed[0].lot_id, "L3");
        assert_eq!(result.lots_consumed[0].quantity, Decimal::new(15, 0));
    }

    #[test]
    fn fifo_gain_calculation() {
        let lots = vec![make_lot("L1", "10", "100", "2023-01-15")];
        let result = dispose_fifo(
            &lots,
            Decimal::new(10, 0),
            Decimal::new(150, 0),
            "2024-06-01",
        );

        // Bought at 100, sold at 150, qty 10 → gain = 500
        assert_eq!(result.total_cost_basis, Decimal::new(1000, 0));
        assert_eq!(result.total_proceeds, Decimal::new(1500, 0));
        assert_eq!(result.realized_gain, Decimal::new(500, 0));
    }

    #[test]
    fn average_cost_method() {
        let lots = sample_lots();
        // Total: 10*100 + 20*150 + 15*200 = 1000 + 3000 + 3000 = 7000
        // Total qty: 45
        // Avg cost: 7000/45 ≈ 155.56
        let result = dispose_average(
            &lots,
            Decimal::new(10, 0),
            Decimal::new(250, 0),
            "2024-06-01",
        );

        assert_eq!(result.lots_consumed.len(), 1);
        assert_eq!(result.total_proceeds, Decimal::new(2500, 0));
        // Cost basis ≈ 155.56 * 10 ≈ 1555.56
        assert!(result.total_cost_basis > Decimal::new(1555, 0));
        assert!(result.total_cost_basis < Decimal::new(1556, 0));
    }

    #[test]
    fn specific_id_method() {
        let lots = sample_lots();
        let result = dispose_specific(
            &lots[1],
            Decimal::new(5, 0),
            Decimal::new(250, 0),
            "2024-06-01",
        );

        assert_eq!(result.lots_consumed[0].lot_id, "L2");
        assert_eq!(result.lots_consumed[0].quantity, Decimal::new(5, 0));
        // Cost: 5 * 150 = 750, Proceeds: 5 * 250 = 1250, Gain: 500
        assert_eq!(result.total_cost_basis, Decimal::new(750, 0));
        assert_eq!(result.realized_gain, Decimal::new(500, 0));
    }

    #[test]
    fn short_term_classification() {
        assert_eq!(
            classify_term("2024-01-15", "2024-06-15"),
            HoldingTerm::ShortTerm
        );
    }

    #[test]
    fn long_term_classification() {
        assert_eq!(
            classify_term("2023-01-15", "2024-06-15"),
            HoldingTerm::LongTerm
        );
    }

    #[test]
    fn unrealized_gain_calculation() {
        let lots = vec![make_lot("L1", "10", "100", "2023-01-15")];
        let gain = unrealized_gain(&lots, Decimal::new(150, 0));
        assert_eq!(gain, Decimal::new(500, 0)); // (150-100) * 10
    }

    #[test]
    fn unrealized_loss_calculation() {
        let lots = vec![make_lot("L1", "10", "100", "2023-01-15")];
        let gain = unrealized_gain(&lots, Decimal::new(80, 0));
        assert_eq!(gain, Decimal::new(-200, 0)); // (80-100) * 10
    }

    #[test]
    fn partial_lot_disposal() {
        let lots = vec![make_lot("L1", "100", "50", "2023-01-15")];
        let result = dispose_fifo(
            &lots,
            Decimal::new(30, 0),
            Decimal::new(75, 0),
            "2024-06-01",
        );

        assert_eq!(result.lots_consumed[0].quantity, Decimal::new(30, 0));
        assert_eq!(result.total_cost_basis, Decimal::new(1500, 0)); // 30 * 50
        assert_eq!(result.total_proceeds, Decimal::new(2250, 0)); // 30 * 75
    }

    #[test]
    fn disposed_lots_excluded() {
        let mut lots = sample_lots();
        lots[0].disposal_date = Some("2024-01-01".into());

        let result = dispose_fifo(
            &lots,
            Decimal::new(5, 0),
            Decimal::new(250, 0),
            "2024-06-01",
        );
        // L1 is disposed, so FIFO starts at L2
        assert_eq!(result.lots_consumed[0].lot_id, "L2");
    }
}
