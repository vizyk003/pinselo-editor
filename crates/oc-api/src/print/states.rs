//! Port of services/api/src/print/states.ts — the print order state machine (A18).
//!
//! ```text
//!   pending_payment ─▶ paid ─▶ rendering ─▶ submitted ─▶ in_production ─▶ shipped ─▶ delivered
//!          │             │          │            │              │             └─▶ returned
//!          └▶ cancelled ◀┴──────────┴─ failed ◀──┴──────────────┘
//! ```
//!
//! `failed` can be retried (back to rendering). Fulfillers may skip steps (submitted → shipped, → delivered).

pub const ORDER_STATUSES: [&str; 10] = ["pending_payment", "paid", "rendering", "submitted", "in_production", "shipped", "delivered", "returned", "failed", "cancelled"];

fn next(from: &str) -> &'static [&'static str] {
    match from {
        "pending_payment" => &["paid", "cancelled"],
        "paid" => &["rendering", "cancelled", "failed"],
        "rendering" => &["submitted", "failed", "cancelled"],
        "submitted" => &["in_production", "shipped", "delivered", "failed", "cancelled"],
        "in_production" => &["shipped", "delivered", "failed", "cancelled"],
        "shipped" => &["delivered", "returned"],
        "delivered" => &["returned"],
        "failed" => &["rendering", "cancelled"],
        _ => &[],
    }
}

pub fn can_transition(from: &str, to: &str) -> bool {
    next(from).contains(&to)
}

/// `TransitionError`: an illegal move (answered 409 `bad_transition` by the routes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionError {
    pub from: String,
    pub to: String,
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `replace('_', ' ')` replaces the first underscore only.
        write!(f, "An order that is {} can’t become {}.", self.from.replacen('_', " ", 1), self.to.replacen('_', " ", 1))
    }
}

pub fn assert_transition(from: &str, to: &str) -> Result<(), TransitionError> {
    if can_transition(from, to) { Ok(()) } else { Err(TransitionError { from: from.into(), to: to.into() }) }
}

/// Next statuses staff can set by hand from the console.
pub fn staff_choices(from: &str) -> Vec<&'static str> {
    next(from).to_vec()
}

/// Customers can cancel only before anything was sent to production.
pub fn customer_can_cancel(s: &str) -> bool {
    matches!(s, "pending_payment" | "paid" | "failed")
}

fn rank(s: &str) -> f64 {
    match s {
        "pending_payment" => 0.0,
        "paid" => 1.0,
        "rendering" => 2.0,
        "submitted" => 3.0,
        "in_production" => 4.0,
        "shipped" => 5.0,
        "delivered" => 6.0,
        "returned" => 7.0,
        "failed" => 3.5,
        _ => 8.0,
    }
}

/// Ignore stale fulfiller updates arriving out of order.
pub fn is_forward(from: &str, to: &str) -> bool {
    can_transition(from, to) && (rank(to) > rank(from) || from == "failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine() {
        assert!(can_transition("pending_payment", "paid"));
        assert!(!can_transition("shipped", "cancelled"));
        assert_eq!(TransitionError { from: "pending_payment".into(), to: "in_production".into() }.to_string(), "An order that is pending payment can’t become in production.");
        assert!(is_forward("failed", "rendering"));
        assert!(!is_forward("submitted", "failed") || rank("failed") > rank("submitted"));
        assert!(customer_can_cancel("failed") && !customer_can_cancel("submitted"));
    }
}
