//! The calculator: a request in, a plan out.

use super::types::*;

/// Work out what it takes to make `req.quantity` of `req.item`.
pub fn calculate(_ds: &Dataset, _req: &Request) -> Result<Plan, PlanError> {
    Err(PlanError("not written yet".into()))
}
