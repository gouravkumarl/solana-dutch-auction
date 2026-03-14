use anchor_lang::prelude::*;

/// The primary auction account.
///
/// Web2 analogy: a row in an `auctions` table.
/// On Solana: a PDA account at address derived from
///   ["auction", auction_id_bytes, seller_pubkey]
///
/// This means:
///   - A seller can run many auctions (different IDs)
///   - The address is deterministic — clients can compute it
///     without querying a database
///   - No central registry needed; the PDA IS the record
#[account]
#[derive(Debug)]
pub struct AuctionState {
    pub auction_id:    u64,           // 8
    pub seller:        Pubkey,        // 32
    pub start_price:   u64,           // 8   lamports
    pub floor_price:   u64,           // 8   lamports
    pub start_slot:    u64,           // 8
    pub end_slot:      u64,           // 8
    pub decay_slots:   u64,           // 8   slots between each price step
    pub price_steps:   u64,           // 8   total number of drops
    pub step_size:     u64,           // 8   lamports per step
    pub title:         String,        // 4 + 64
    pub status:        AuctionStatus, // 1
    pub winner:        Option<Pubkey>,// 1 + 32
    pub winning_price: u64,           // 8
    pub bid_count:     u32,           // 4
    pub created_at:    i64,           // 8
    pub bump:          u8,            // 1
    pub vault_bump:    u8,            // 1
}

impl AuctionState {
    pub const LEN: usize = 8   // discriminator
        + 8   // auction_id
        + 32  // seller
        + 8   // start_price
        + 8   // floor_price
        + 8   // start_slot
        + 8   // end_slot
        + 8   // decay_slots
        + 8   // price_steps
        + 8   // step_size
        + 4 + 64 // title
        + 1   // status
        + 1 + 32 // winner (Option<Pubkey>)
        + 8   // winning_price
        + 4   // bid_count
        + 8   // created_at
        + 1   // bump
        + 1;  // vault_bump
    // = 8 + 194 = 202 bytes
}

/// Written once when a bid wins. Immutable after creation.
/// Serves as an on-chain receipt — anyone can verify the outcome
/// without trusting the seller or buyer.
///
/// Web2 analogy: an `order_fulfillments` row, or a Stripe
/// payment intent record. On Solana this is publicly verifiable
/// cryptographic proof of the trade.
#[account]
#[derive(Debug)]
pub struct SettlementRecord {
    pub auction_id:   u64,    // 8
    pub winner:       Pubkey, // 32
    pub seller:       Pubkey, // 32
    pub price_paid:   u64,    // 8
    pub overpayment:  u64,    // 8   refunded instantly
    pub winning_slot: u64,    // 8
    pub settled_at:   i64,    // 8
    pub bump:         u8,     // 1
}

impl SettlementRecord {
    pub const LEN: usize = 8 + 8 + 32 + 32 + 8 + 8 + 8 + 8 + 1; // = 113 bytes
}

/// Auction lifecycle.
///
/// The state machine is intentionally simple and linear — once
/// Sold, Expired, or Cancelled, there is no going back.
/// This makes the program logic auditable and easy to reason about.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, Debug)]
pub enum AuctionStatus {
    Pending,   // created, not yet at start_slot
    Active,    // start_slot reached, accepting bids
    Sold,      // a bid won — terminal
    Expired,   // end_slot passed with no winner — terminal
    Cancelled, // seller cancelled before any bid — terminal
}
