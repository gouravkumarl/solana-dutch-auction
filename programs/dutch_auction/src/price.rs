use anchor_lang::prelude::*;
use crate::errors::AuctionError;

/// Compute the current auction price from on-chain state.
///
/// This function is the core of why this system is trustless.
/// In Web2, a server ticks the price down on a timer and writes
/// the new value to a database. Any outage, bug, or manipulation
/// by the operator changes the price.
///
/// Here, price is a pure function of:
///   - parameters set at auction creation (immutable on-chain)
///   - the current slot number (provided by the validator, consensus-agreed)
///
/// Anyone running this function with the same inputs gets the same
/// output. There is no server, no mutable price field, no trust.
///
/// Formula:
///   steps_elapsed = min((current_slot - start_slot) / decay_slots, price_steps)
///   current_price = start_price - (steps_elapsed * step_size)
///
/// Edge cases handled:
///   - current_slot < start_slot  → returns start_price
///   - steps_elapsed >= price_steps → returns floor_price exactly
///   - integer division truncates  → price drops at discrete slot boundaries
pub fn compute_current_price(
    start_price: u64,
    floor_price: u64,
    start_slot: u64,
    decay_slots: u64,
    price_steps: u64,
    step_size: u64,
    current_slot: u64,
) -> Result<u64> {
    // Auction hasn't started yet
    if current_slot < start_slot {
        return Ok(start_price);
    }

    let slots_elapsed = current_slot
        .checked_sub(start_slot)
        .ok_or(AuctionError::MathOverflow)?;

    // Integer division: price steps down at each boundary
    let steps_elapsed = slots_elapsed / decay_slots;

    // Cap at max steps (floor_price is the minimum)
    let steps_elapsed = steps_elapsed.min(price_steps);

    let total_drop = step_size
        .checked_mul(steps_elapsed)
        .ok_or(AuctionError::MathOverflow)?;

    let price = start_price
        .checked_sub(total_drop)
        .unwrap_or(floor_price) // safety net: never go below floor
        .max(floor_price);      // enforce floor

    Ok(price)
}

/// Preview: return the price at a specific future slot.
/// Used by the CLI/frontend to show buyers the price schedule.
pub fn price_at_slot(
    start_price: u64,
    floor_price: u64,
    start_slot: u64,
    decay_slots: u64,
    price_steps: u64,
    step_size: u64,
    target_slot: u64,
) -> u64 {
    compute_current_price(
        start_price,
        floor_price,
        start_slot,
        decay_slots,
        price_steps,
        step_size,
        target_slot,
    )
    .unwrap_or(floor_price)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(current_slot: u64) -> u64 {
        // start=1000, floor=100, start_slot=0, decay=10slots,
        // steps=9, step_size=100
        compute_current_price(1000, 100, 0, 10, 9, 100, current_slot).unwrap()
    }

    #[test]
    fn starts_at_start_price() {
        assert_eq!(price(0), 1000);
    }

    #[test]
    fn before_start_returns_start_price() {
        // current_slot < start_slot edge case
        let p = compute_current_price(1000, 100, 50, 10, 9, 100, 30).unwrap();
        assert_eq!(p, 1000);
    }

    #[test]
    fn drops_after_first_decay_period() {
        assert_eq!(price(10), 900);
        assert_eq!(price(11), 900); // still same step
        assert_eq!(price(19), 900); // not yet next step
    }

    #[test]
    fn drops_at_each_boundary() {
        assert_eq!(price(20), 800);
        assert_eq!(price(30), 700);
        assert_eq!(price(40), 600);
        assert_eq!(price(50), 500);
    }

    #[test]
    fn floors_at_floor_price() {
        assert_eq!(price(90),  100); // last step
        assert_eq!(price(200), 100); // well past end — still floor
        assert_eq!(price(u64::MAX / 2), 100); // extreme
    }

    #[test]
    fn never_goes_below_floor() {
        for slot in 0..200 {
            let p = price(slot);
            assert!(p >= 100, "price {} below floor at slot {}", p, slot);
            assert!(p <= 1000, "price {} above start at slot {}", p, slot);
        }
    }

    #[test]
    fn monotonically_decreasing() {
        let mut prev = price(0);
        for slot in 1..200 {
            let curr = price(slot);
            assert!(curr <= prev, "price increased at slot {}: {} > {}", slot, curr, prev);
            prev = curr;
        }
    }
}
