use anchor_lang::prelude::*;

#[event]
pub struct AuctionCreated {
    pub auction_id:  u64,
    pub seller:      Pubkey,
    pub start_price: u64,
    pub floor_price: u64,
    pub start_slot:  u64,
    pub end_slot:    u64,
    pub title:       String,
}

/// Emitted the moment a winning bid lands.
/// Contains enough data to reconstruct the full trade without
/// querying any additional accounts.
#[event]
pub struct BidWon {
    pub auction_id:   u64,
    pub winner:       Pubkey,
    pub price_paid:   u64,
    pub overpayment:  u64,
    pub winning_slot: u64,
}

#[event]
pub struct AuctionCancelled {
    pub auction_id:   u64,
    pub cancelled_by: Pubkey,
}

#[event]
pub struct AuctionExpired {
    pub auction_id: u64,
    pub seller:     Pubkey,
    pub end_slot:   u64,
}
