# On-Chain Dutch Auction Engine

A Dutch auction where the price is a **pure function of the Solana slot clock** — no server, no price oracle, no trusted auctioneer.

[![Anchor](https://img.shields.io/badge/Anchor-0.30.1-purple)](https://www.anchor-lang.com/)
[![Solana Devnet](https://img.shields.io/badge/deployed-Devnet-green)](https://explorer.solana.com/?cluster=devnet)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)

---

## Deployment

| | |
|---|---|
| **Program ID** | `DAuct1onXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX` |
| **Network** | Solana Devnet |
| **Explorer** | [View Program](https://explorer.solana.com/address/DAuct1onXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX?cluster=devnet) |

### Devnet Transaction Links

| | |
|---|---|
| Deploy | `<TX>` |
| Create auction | `<TX>` |
| Winning bid (with overpayment refund) | `<TX>` |
| Settle expired | `<TX>` |

---

## What is a Dutch Auction?

A Dutch auction opens at a **high price** and falls on a fixed schedule until someone buys or it expires. The first valid bid wins immediately — there is no bidding war, no waiting period, no second-price mechanism.

Real-world uses: Google's 2004 IPO, ICANN domain auctions, flower markets in the Netherlands (hence the name), NFT drops, DeFi token launches.

---

## How This Works in Web2

```
                    ┌───────────────────────────────────┐
                    │         Auction Service            │
                    │                                    │
  ┌────────┐  POST  │  Node.js / Rails                   │
  │ Seller │───────►│  • creates auction row in DB       │
  └────────┘        │  • starts a cron job / setTimeout  │
                    │    every N seconds:                 │
                    │      price -= step_size             │
                    │      db.update(auction)             │
                    │      if price <= floor: close()    │
  ┌────────┐  POST  │                                    │
  │ Bidder │───────►│  • checks current price from DB    │
  └────────┘        │  • if bid >= price:                 │
                    │      charge via Stripe              │
                    │      mark auction "sold"            │
                    │      pay seller via ACH             │
                    └───────────────────────────────────┘
                                    │
                           ┌────────▼────────┐
                           │   PostgreSQL     │
                           │                 │
                           │  auctions       │
                           │  current_price  │ ← mutated by cron
                           │  status         │
                           └─────────────────┘
```

**The server is the auctioneer.** It holds the current price, mutates it on a timer, and you have to trust it:
- Did it update the price at the right time?
- Is it showing everyone the same price?
- Can it freeze your bid or censor a winning buyer?
- What if it goes offline mid-auction?

---

## How This Works on Solana

```
                    ┌───────────────────────────────────────┐
                    │         Dutch Auction Program          │
                    │         (BPF, deployed on-chain)       │
                    │                                        │
  ┌────────┐  sign  │  create_auction                        │
  │ Seller │───────►│    • writes AuctionState PDA           │
  └────────┘        │    • stores: start_price, floor_price, │
                    │      start_slot, decay_slots,          │
                    │      price_steps, step_size            │
                    │    • these values are now IMMUTABLE    │
                    │                                        │
  ┌────────┐  sign  │  place_bid                             │
  │ Bidder │───────►│    • reads Clock::get().slot           │
  └────────┘        │    • computes price (pure function)    │
                    │    • if bid >= price → winner           │
                    │    • transfers SOL atomically          │
                    │    • writes SettlementRecord PDA       │
                    └───────────────────────────────────────┘
                                    │
              ┌─────────────────────┴────────────────────────┐
              │                                              │
     ┌────────▼──────────┐                    ┌─────────────▼──────────┐
     │   AuctionState     │                    │    SOL Vault PDA       │
     │   PDA              │                    │                        │
     │                    │                    │  holds zero bytes      │
     │  start_price: u64  │                    │  holds actual lamports │
     │  floor_price: u64  │                    │  during bid transit    │
     │  start_slot:  u64  │                    │                        │
     │  decay_slots: u64  │                    │  signed only by        │
     │  price_steps: u64  │                    │  program via seeds     │
     │  step_size:   u64  │                    └────────────────────────┘
     │  status: enum      │
     │  winner: Option    │
     └────────────────────┘
```

**There is no auctioneer.** The price is not stored anywhere. It is *computed* from:

1. Parameters fixed at auction creation (immutable, on-chain)
2. The current Solana slot (provided by the validator network under BFT consensus)

```rust
// The entire price logic — no server, no cron, no database
fn compute_current_price(start_price, floor_price, start_slot,
                          decay_slots, price_steps, step_size,
                          current_slot) -> u64 {
    let slots_elapsed = current_slot - start_slot;
    let steps_elapsed = min(slots_elapsed / decay_slots, price_steps);
    let price = start_price - (steps_elapsed * step_size);
    max(price, floor_price)
}
```

Any participant — the seller, the buyer, a third-party auditor — can run this function and get the same answer. It is deterministic and public.

---

## Architecture: The Key Design Decisions

### 1. Price as a function, not a field

The most important design choice: **current_price is never stored**.

In Web2, you'd have a `current_price` column that a cron job updates. On Solana, storing and updating this field would require a signed transaction every N slots — expensive and fragile. Instead, `current_price` is derived on-demand inside `place_bid`. This means:

- No maintenance transactions required
- Price is always consistent — no race condition where a bid lands between a cron update
- The seller cannot manipulate the price after creation

### 2. Three-PDA account chain

```
AuctionState PDA          SOL Vault PDA           SettlementRecord PDA
["auction", id, seller]   ["vault", id]            ["settlement", id]

Stores metadata           Holds lamports           Written once at win
Readable by anyone        Program-controlled       Immutable receipt
202 bytes                 0 bytes (lamports only)  113 bytes
~0.0015 SOL rent          ~0.0009 SOL rent         ~0.001 SOL rent
```

This mirrors the Web2 separation of concerns (auctions table / vaults table / fulfillments table) but enforces it cryptographically. The vault cannot be drained without a valid program instruction. The settlement record cannot be forged.

### 3. Atomic win + overpayment refund

In one transaction:
1. Buyer sends `bid_amount` to vault
2. Program computes `current_price` from clock
3. If `bid_amount >= current_price`: vault pays seller `current_price`, vault refunds buyer `bid_amount - current_price`
4. SettlementRecord written

No separate "refund" transaction. No possibility of the buyer paying more than the price. Atomic means either all of this happens or none of it does.

### 4. Slot-based timing vs timestamp-based

We use **slots** not Unix timestamps. Why?

- `Clock::get().unix_timestamp` on Solana can drift slightly — validators may produce timestamps that are a few seconds off
- `Clock::get().slot` is strictly monotonically increasing and manipulation-resistant
- Slot timing is ~400ms per slot, predictable, and tied to consensus
- A seller can quote "price drops every 150 slots (~60 seconds)" and that is a precise commitment

---

## Price Schedule Example

```
Auction: 10 SOL → 1 SOL over 9 steps, 150 slots per step (~60 seconds each)

Step  Slot   Price
────  ─────  ──────────────────────────────────────────
  0    0     10.0 SOL  ██████████
  1  150      9.0 SOL  █████████░
  2  300      8.0 SOL  ████████░░
  3  450      7.0 SOL  ███████░░░
  4  600      6.0 SOL  ██████░░░░
  5  750      5.0 SOL  █████░░░░░
  6  900      4.0 SOL  ████░░░░░░
  7 1050      3.0 SOL  ███░░░░░░░
  8 1200      2.0 SOL  ██░░░░░░░░
  9 1350      1.0 SOL  █░░░░░░░░░  ← floor (auction expires)

Total duration: 1350 slots ≈ 9 minutes
```

---

## Web2 → Solana Translation Table

| Web2 Concept | Solana Implementation | Why it works |
|---|---|---|
| `auctions` DB table | `AuctionState` PDA | Address derived from `["auction", id, seller]` — deterministic, no registry needed |
| `current_price` column | Pure function of slot | Never stored; computed on read; impossible to manipulate |
| Cron job / setTimeout | `Clock::get().slot` | BFT consensus provides tamper-resistant time |
| Stripe / bank account | SOL Vault PDA | Lamports held by PDA; only program can sign withdrawals |
| Fulfillment record | `SettlementRecord` PDA | Written atomically with the win; cryptographic proof |
| JWT / session auth | `Signer<'info>` | Private key signature verified by runtime, not application code |
| Database transaction | Solana transaction | Atomic by design; all instructions succeed or all fail |
| REST API endpoint | Anchor instruction | On-chain, permissionless, callable by anyone with correct accounts |
| Admin role | `seller: Pubkey` field | Stored at creation; runtime checks `signer.key() == auction.seller` |

---

## Known Attack: Slot-Boundary Sniping

Because price drops at discrete slot boundaries, a sophisticated bidder can exploit the schedule:

1. They watch the current slot in real time
2. At the exact moment a new decay period starts, they submit a bid at the new (lower) price
3. If their transaction confirms in that slot, they win at a price the seller did not intend to accept yet

**How severe is this?** On Solana, slots are ~400ms. A bidder sniping slot boundary N gets the same price as a bidder who arrived 2 seconds later — it's not a fundamental economic exploit, just a first-mover advantage within a single step.

**Mitigations (not implemented here — tradeoffs noted):**

| Mitigation | How it works | Cost |
|---|---|---|
| Commit-reveal bidding | Bidder submits a hash first, reveals later | Two-transaction UX, adds latency |
| Randomized decay windows | `decay_slots += random(0, jitter)` | Requires VRF oracle; complex |
| Continuous price (no steps) | Price decays by lamport per slot | Bidder can never know "the" price; worse UX |
| Accept sniping | It is rational market behavior | Nothing — this is actually fine for most use cases |

For most auction scenarios — NFT drops, asset sales, token launches — slot-boundary sniping is acceptable. The seller sets a price schedule they are willing to accept at each step. A sniper buying at step N is buying at exactly the price the seller agreed to at that point.

---

## Tradeoffs and Honest Constraints

### What you gain

**Trustlessness.** Once created, an auction runs exactly as specified. The seller cannot change the price schedule, the program cannot be shut down, and no third party can censor a winning bid.

**Composability.** Any other Solana program can call `place_bid` in a CPI. An AMM could automatically bid when the price crosses a threshold. A DAO could run auctions for treasury assets. None of this requires permission.

**Global settlement.** A buyer in Singapore and a seller in Brazil settle in 400ms with no correspondent banks, currency conversion delays, or wire transfer fees.

### What you give up

**Privacy.** Every bid, price, and winner is publicly visible on-chain. There is no way to run a sealed-bid auction using this model without zero-knowledge proofs.

**Flexibility after deploy.** If the price schedule has a bug, you cannot patch it. The auction runs as written. This forces you to get it right at creation time, which is a higher standard than editing a database row.

**Granularity.** Price drops at discrete slot boundaries. There is no sub-slot price — the price is the same for slot 150 through slot 299. In a high-frequency trading context, this could be exploited by bidders who watch the mempool and snipe at the exact moment a slot boundary passes.

**Storage costs real money.** The three PDAs cost ~0.0034 SOL in rent (~$0.68 at $200/SOL). A Web2 database row costs fractions of a cent. For high-volume applications, this matters.

**No automatic expiry.** When the auction ends with no winner, nothing happens automatically. Someone must call `settle_expired`. In Web2, a cron job handles this. On Solana, you need Clockwork or an off-chain keeper — or just accept that the account sits idle.

---

## Quick Start

```bash
# Prerequisites: Rust, Solana CLI ≥1.18, Anchor ≥0.30.1, Node.js ≥20

git clone https://github.com/YOUR_HANDLE/solana-dutch-auction
cd solana-dutch-auction
yarn install

# Build
anchor build

# Get your real program ID after build
solana address -k target/deploy/dutch_auction-keypair.json
# → Update declare_id!() in lib.rs and Anchor.toml, then anchor build again

# Test (local validator)
anchor test

# Deploy to Devnet
solana config set --url devnet
solana airdrop 2
anchor deploy --provider.cluster devnet
```

---

## CLI Usage

```bash
# Create a 10 SOL → 1 SOL auction over 9 steps, 150 slots per step
yarn cli create \
  --id 1 \
  --start-price 10 \
  --floor-price 1 \
  --decay-slots 150 \
  --price-steps 9 \
  --title "Rare item auction"

# View live state and current price
yarn cli status --id 1 --seller <YOUR_PUBKEY>

# Print full price schedule
yarn cli schedule --id 1 --seller <YOUR_PUBKEY>

# Place a bid (wins if amount >= current price, refunds overpayment)
yarn cli bid --id 1 --seller <SELLER_PUBKEY> --amount 7.5

# Cancel (seller only, before any bid)
yarn cli cancel --id 1
```

---

## Testing

```bash
anchor test
```

The test suite verifies:
- Correct on-chain state after `create_auction`
- Winning bid at exact `start_price`
- Overpayment refunded precisely to the lamport
- Underbid rejected with `BidTooLow`
- `cancel_auction` permissions (seller vs stranger)
- Price formula properties: monotonically decreasing, never below floor, stable within decay periods
- `AuctionState` account byte sizing
- `SettlementRecord` written correctly with all fields

---

## Security Notes

- **Overflow protection**: `overflow-checks = true` in release profile; all arithmetic uses `checked_*` or explicit bounds
- **Floor enforcement**: `max(computed_price, floor_price)` in price function — mathematical guarantee, not just a condition
- **Atomic settlement**: win + refund + seller payment in one transaction; no partial execution possible
- **PDA authority**: vault is a `SystemAccount` PDA; only the program, signing with known seeds, can move its lamports
- **Immutable auction parameters**: all pricing fields set at `create_auction`, never written again
- **Terminal states**: `Sold`, `Expired`, `Cancelled` — program rejects all instructions once these are set

---

## License

MIT
