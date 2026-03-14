use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("DAuct1onXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX");

pub mod state;
pub mod errors;
pub mod events;
pub mod price;

use state::*;
use errors::AuctionError;
use events::*;
use price::compute_current_price;

#[program]
pub mod dutch_auction {
    use super::*;

    pub fn create_auction(
        ctx: Context<CreateAuction>,
        auction_id: u64,
        start_price: u64,
        floor_price: u64,
        start_slot: u64,
        decay_slots: u64,
        price_steps: u64,
        title: String,
    ) -> Result<()> {
        require!(start_price > floor_price, AuctionError::InvalidPriceRange);
        require!(floor_price > 0, AuctionError::InvalidPriceRange);
        require!(decay_slots > 0, AuctionError::InvalidDecaySlots);
        require!(price_steps > 0 && price_steps <= 1000, AuctionError::InvalidPriceSteps);
        require!(title.len() <= 64, AuctionError::TitleTooLong);

        let clock = Clock::get()?;
        require!(start_slot >= clock.slot, AuctionError::StartSlotInPast);

        let total_slots = decay_slots
            .checked_mul(price_steps)
            .ok_or(AuctionError::MathOverflow)?;
        let end_slot = start_slot
            .checked_add(total_slots)
            .ok_or(AuctionError::MathOverflow)?;
        let step_size = (start_price - floor_price) / price_steps;
        require!(step_size > 0, AuctionError::StepSizeTooSmall);

        let auction = &mut ctx.accounts.auction;
        auction.auction_id    = auction_id;
        auction.seller        = ctx.accounts.seller.key();
        auction.start_price   = start_price;
        auction.floor_price   = floor_price;
        auction.start_slot    = start_slot;
        auction.end_slot      = end_slot;
        auction.decay_slots   = decay_slots;
        auction.price_steps   = price_steps;
        auction.step_size     = step_size;
        auction.title         = title.clone();
        auction.status        = AuctionStatus::Pending;
        auction.winner        = None;
        auction.winning_price = 0;
        auction.bid_count     = 0;
        auction.created_at    = clock.unix_timestamp;
        auction.bump          = ctx.bumps.auction;
        auction.vault_bump    = ctx.bumps.vault;

        emit!(AuctionCreated {
            auction_id,
            seller: auction.seller,
            start_price,
            floor_price,
            start_slot,
            end_slot,
            title,
        });
        Ok(())
    }

    pub fn place_bid(ctx: Context<PlaceBid>, bid_amount: u64) -> Result<()> {
        let auction      = &ctx.accounts.auction;
        let clock        = Clock::get()?;
        let current_slot = clock.slot;

        require!(current_slot >= auction.start_slot, AuctionError::AuctionNotStarted);
        require!(current_slot <= auction.end_slot,   AuctionError::AuctionExpired);

        let current_price = compute_current_price(
            auction.start_price,
            auction.floor_price,
            auction.start_slot,
            auction.decay_slots,
            auction.price_steps,
            auction.step_size,
            current_slot,
        )?;

        require!(bid_amount >= current_price, AuctionError::BidTooLow);

        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.bidder.to_account_info(),
                    to:   ctx.accounts.vault.to_account_info(),
                },
            ),
            bid_amount,
        )?;

        let auction_id_bytes = auction.auction_id.to_le_bytes();
        let vault_seeds: &[&[u8]] = &[
            b"vault",
            auction_id_bytes.as_ref(),
            &[auction.vault_bump],
        ];

        let overpayment = bid_amount
            .checked_sub(current_price)
            .ok_or(AuctionError::MathOverflow)?;

        if overpayment > 0 {
            pda_transfer(
                &ctx.accounts.vault.to_account_info(),
                &ctx.accounts.bidder.to_account_info(),
                &ctx.accounts.system_program.to_account_info(),
                overpayment,
                vault_seeds,
            )?;
        }

        pda_transfer(
            &ctx.accounts.vault.to_account_info(),
            &ctx.accounts.seller.to_account_info(),
            &ctx.accounts.system_program.to_account_info(),
            current_price,
            vault_seeds,
        )?;

        let settlement         = &mut ctx.accounts.settlement;
        settlement.auction_id  = auction.auction_id;
        settlement.winner      = ctx.accounts.bidder.key();
        settlement.seller      = auction.seller;
        settlement.price_paid  = current_price;
        settlement.overpayment = overpayment;
        settlement.winning_slot = current_slot;
        settlement.settled_at  = clock.unix_timestamp;
        settlement.bump        = ctx.bumps.settlement;

        let auction           = &mut ctx.accounts.auction;
        auction.status        = AuctionStatus::Sold;
        auction.winner        = Some(ctx.accounts.bidder.key());
        auction.winning_price = current_price;
        auction.bid_count     = auction.bid_count.saturating_add(1);

        emit!(BidWon {
            auction_id:   auction.auction_id,
            winner:       ctx.accounts.bidder.key(),
            price_paid:   current_price,
            overpayment,
            winning_slot: current_slot,
        });

        Ok(())
    }

    pub fn cancel_auction(ctx: Context<CancelAuction>) -> Result<()> {
        let auction = &ctx.accounts.auction;
        require!(auction.winner.is_none(), AuctionError::CannotCancel);
        require!(
            auction.status == AuctionStatus::Pending
            || auction.status == AuctionStatus::Active,
            AuctionError::CannotCancel
        );
        emit!(AuctionCancelled {
            auction_id:   auction.auction_id,
            cancelled_by: ctx.accounts.seller.key(),
        });
        Ok(())
    }

    pub fn settle_expired(ctx: Context<SettleExpired>) -> Result<()> {
        let auction = &ctx.accounts.auction;
        let clock   = Clock::get()?;
        require!(clock.slot > auction.end_slot, AuctionError::AuctionNotExpired);
        require!(
            auction.status != AuctionStatus::Sold
            && auction.status != AuctionStatus::Expired
            && auction.status != AuctionStatus::Cancelled,
            AuctionError::AlreadySettled
        );
        emit!(AuctionExpired {
            auction_id: auction.auction_id,
            seller:     auction.seller,
            end_slot:   auction.end_slot,
        });
        Ok(())
    }
}

fn pda_transfer<'info>(
    from:           &AccountInfo<'info>,
    to:             &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    amount:         u64,
    seeds:          &[&[u8]],
) -> Result<()> {
    anchor_lang::solana_program::program::invoke_signed(
        &anchor_lang::solana_program::system_instruction::transfer(
            from.key,
            to.key,
            amount,
        ),
        &[from.clone(), to.clone(), system_program.clone()],
        &[seeds],
    )?;
    Ok(())
}

// ── Account Contexts ──────────────────────────────────────────────

#[derive(Accounts)]
#[instruction(auction_id: u64)]
pub struct CreateAuction<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(
        init,
        payer  = seller,
        space  = AuctionState::LEN,
        seeds  = [b"auction", auction_id.to_le_bytes().as_ref(), seller.key().as_ref()],
        bump,
    )]
    pub auction: Account<'info, AuctionState>,

    /// Vault is a zero-data PDA owned by System Program.
    /// It transiently holds lamports during bid settlement.
    /// Only the program, signing with [b"vault", auction_id], can move them.
    #[account(
        init,
        payer  = seller,
        space  = 0,
        seeds  = [b"vault", auction_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub vault: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct PlaceBid<'info> {
    #[account(mut)]
    pub bidder: Signer<'info>,

    /// Seller does not sign bid transactions — they are the recipient.
    /// The constraint `seller.key() == auction.seller` ensures the client
    /// cannot pass a substitute account to redirect proceeds.
    #[account(
        mut,
        constraint = seller.key() == auction.seller @ AuctionError::Unauthorized,
    )]
    pub seller: SystemAccount<'info>,

    /// Terminal-status constraints run at account validation time,
    /// before compute budget is consumed by instruction logic.
    /// Temporal checks (start_slot, end_slot) live inside the instruction
    /// because Clock::get() is unavailable in constraint expressions.
    ///
    /// close = seller: on success Anchor zeroes this account and
    /// transfers its rent-exempt lamports (~0.0015 SOL) to the seller.
    #[account(
        mut,
        constraint = auction.status != AuctionStatus::Sold      @ AuctionError::AuctionNotBiddable,
        constraint = auction.status != AuctionStatus::Expired   @ AuctionError::AuctionExpired,
        constraint = auction.status != AuctionStatus::Cancelled @ AuctionError::AuctionNotBiddable,
        close = seller,
    )]
    pub auction: Account<'info, AuctionState>,

    #[account(
        mut,
        seeds = [b"vault", auction.auction_id.to_le_bytes().as_ref()],
        bump  = auction.vault_bump,
    )]
    pub vault: SystemAccount<'info>,

    #[account(
        init,
        payer  = bidder,
        space  = SettlementRecord::LEN,
        seeds  = [b"settlement", auction.auction_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub settlement: Account<'info, SettlementRecord>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CancelAuction<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    /// close = seller: AuctionState is deallocated on success;
    /// rent returned to seller (~0.0015 SOL).
    /// The seller constraint prevents any other signer from triggering
    /// the close and redirecting the rent.
    #[account(
        mut,
        constraint = auction.seller == seller.key() @ AuctionError::Unauthorized,
        close = seller,
    )]
    pub auction: Account<'info, AuctionState>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SettleExpired<'info> {
    /// Permissionless: any account may call settle_expired to clean
    /// up an expired auction. The caller pays the tx fee but earns
    /// nothing — this is a public-good keeper instruction.
    #[account(mut)]
    pub caller: Signer<'info>,

    /// close = seller: rent goes to the original seller, not the caller.
    /// This removes any economic incentive for griefing.
    #[account(
        mut,
        close = seller,
    )]
    pub auction: Account<'info, AuctionState>,

    /// CHECK: receives rent from the closed AuctionState.
    /// Verified against auction.seller via constraint; this check is
    /// safe because auction is an Anchor-deserialized Account<AuctionState>
    /// whose data has already been ownership- and discriminator-verified.
    #[account(
        mut,
        constraint = seller.key() == auction.seller @ AuctionError::Unauthorized,
    )]
    pub seller: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}
