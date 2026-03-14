use anchor_lang::prelude::*;

#[error_code]
pub enum AuctionError {
    #[msg("start_price must be strictly greater than floor_price, and floor_price must be > 0")]
    InvalidPriceRange,

    #[msg("decay_slots must be at least 1")]
    InvalidDecaySlots,

    #[msg("price_steps must be between 1 and 1000")]
    InvalidPriceSteps,

    #[msg("step_size is too small — increase the price range or reduce price_steps")]
    StepSizeTooSmall,

    #[msg("start_slot must be >= current slot")]
    StartSlotInPast,

    #[msg("Title exceeds 64 bytes")]
    TitleTooLong,

    #[msg("Auction is not open for bidding")]
    AuctionNotBiddable,

    #[msg("Auction has not reached its start slot yet")]
    AuctionNotStarted,

    #[msg("Auction has passed its end slot with no winner")]
    AuctionExpired,

    #[msg("Bid amount is below the current auction price")]
    BidTooLow,

    #[msg("Signer is not authorized for this action")]
    Unauthorized,

    #[msg("Auction cannot be cancelled — it may already have a winner")]
    CannotCancel,

    #[msg("Auction end slot has not passed yet")]
    AuctionNotExpired,

    #[msg("Auction is already in a terminal state")]
    AlreadySettled,

    #[msg("Arithmetic overflow in price calculation")]
    MathOverflow,
}
