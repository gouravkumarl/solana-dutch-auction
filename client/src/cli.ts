#!/usr/bin/env ts-node
/**
 * Dutch Auction Engine — CLI Client
 *
 * Commands:
 *   create   Create a new auction
 *   bid      Place a bid (wins instantly if price met)
 *   cancel   Seller cancels auction
 *   settle   Mark expired auction as settled
 *   status   View current auction state + live price
 *   schedule Print the full price schedule for an auction
 */

import * as anchor from "@coral-xyz/anchor";
import { BN, AnchorProvider } from "@coral-xyz/anchor";
import {
  Connection, Keypair, PublicKey,
  LAMPORTS_PER_SOL, SystemProgram, clusterApiUrl,
} from "@solana/web3.js";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";
import yargs from "yargs";
import { hideBin } from "yargs/helpers";
import chalk from "chalk";
import ora from "ora";

const PROGRAM_ID = new PublicKey("DAuct1onXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX");
const IDL        = require("../../target/idl/dutch_auction.json");

function loadKeypair(p: string): Keypair {
  const resolved = p.startsWith("~") ? path.join(os.homedir(), p.slice(1)) : p;
  return Keypair.fromSecretKey(Buffer.from(JSON.parse(fs.readFileSync(resolved, "utf-8"))));
}

function auctionPDA(id: BN, seller: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("auction"), id.toArrayLike(Buffer, "le", 8), seller.toBuffer()],
    PROGRAM_ID
  );
}

function vaultPDA(id: BN) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), id.toArrayLike(Buffer, "le", 8)],
    PROGRAM_ID
  );
}

function settlementPDA(id: BN) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("settlement"), id.toArrayLike(Buffer, "le", 8)],
    PROGRAM_ID
  );
}

// Mirror of Rust price formula
function computePrice(a: any, currentSlot: number): number {
  const start = a.startSlot.toNumber();
  if (currentSlot < start) return a.startPrice.toNumber();
  const elapsed = currentSlot - start;
  const steps   = Math.min(Math.floor(elapsed / a.decaySlots.toNumber()), a.priceSteps.toNumber());
  const price   = a.startPrice.toNumber() - steps * a.stepSize.toNumber();
  return Math.max(price, a.floorPrice.toNumber());
}

async function setup(walletPath: string) {
  const keypair  = loadKeypair(walletPath);
  const conn     = new Connection(clusterApiUrl("devnet"), "confirmed");
  const wallet   = new anchor.Wallet(keypair);
  const provider = new AnchorProvider(conn, wallet, { commitment: "confirmed" });
  anchor.setProvider(provider);
  const program  = new anchor.Program(IDL, PROGRAM_ID, provider) as any;
  return { program, provider, keypair, conn };
}

function statusLabel(status: any): string {
  const k = Object.keys(status)[0];
  return { pending: "⏳ Pending", active: "🟢 Active", sold: "🎉 Sold", expired: "💀 Expired", cancelled: "❌ Cancelled" }[k] ?? k;
}

function printAuction(a: any, pk: PublicKey, currentPrice?: number, currentSlot?: number) {
  console.log(chalk.bold("\n┌── Dutch Auction ────────────────────────────────────────"));
  console.log(`│  Address      : ${chalk.cyan(pk.toBase58())}`);
  console.log(`│  Title        : ${a.title}`);
  console.log(`│  Status       : ${statusLabel(a.status)}`);
  console.log(`│  Seller       : ${a.seller.toBase58()}`);
  console.log(`│  Start Price  : ${a.startPrice.toNumber() / LAMPORTS_PER_SOL} SOL`);
  console.log(`│  Floor Price  : ${a.floorPrice.toNumber() / LAMPORTS_PER_SOL} SOL`);
  console.log(`│  Step Size    : ${a.stepSize.toNumber() / LAMPORTS_PER_SOL} SOL`);
  console.log(`│  Price Steps  : ${a.priceSteps}`);
  console.log(`│  Decay Slots  : ${a.decaySlots} slots per step (~${(a.decaySlots.toNumber() * 0.4).toFixed(1)}s)`);
  console.log(`│  Start Slot   : ${a.startSlot}`);
  console.log(`│  End Slot     : ${a.endSlot}`);
  if (currentSlot !== undefined && currentPrice !== undefined) {
    const bar = "█".repeat(Math.round(10 * (currentPrice - a.floorPrice.toNumber()) / (a.startPrice.toNumber() - a.floorPrice.toNumber())));
    console.log(`│  Current Slot : ${currentSlot}`);
    console.log(`│  ` + chalk.yellow(`Current Price  : ${currentPrice / LAMPORTS_PER_SOL} SOL  [${bar.padEnd(10, "░")}]`));
  }
  if (a.winner) {
    console.log(`│  Winner       : ${chalk.green(a.winner.toBase58())}`);
    console.log(`│  Price Paid   : ${a.winningPrice.toNumber() / LAMPORTS_PER_SOL} SOL`);
  }
  console.log(`└─────────────────────────────────────────────────────────\n`);
}

// ── CLI ────────────────────────────────────────────────────────────

const argv = yargs(hideBin(process.argv))
  .command("create", "Create a new Dutch auction", y => y
    .option("wallet",       { type: "string", default: "~/.config/solana/id.json" })
    .option("id",           { type: "number", demandOption: true })
    .option("start-price",  { type: "number", demandOption: true, describe: "SOL" })
    .option("floor-price",  { type: "number", demandOption: true, describe: "SOL" })
    .option("start-slot",   { type: "number", default: 0, describe: "0 = current slot" })
    .option("decay-slots",  { type: "number", default: 150 })
    .option("price-steps",  { type: "number", default: 9 })
    .option("title",        { type: "string", default: "Dutch Auction" })
  )
  .command("bid", "Place a bid on an auction", y => y
    .option("wallet",  { type: "string", default: "~/.config/solana/id.json" })
    .option("id",      { type: "number", demandOption: true })
    .option("seller",  { type: "string", demandOption: true })
    .option("amount",  { type: "number", demandOption: true, describe: "SOL to send (>=current price)" })
  )
  .command("cancel", "Cancel your auction", y => y
    .option("wallet", { type: "string", default: "~/.config/solana/id.json" })
    .option("id",     { type: "number", demandOption: true })
  )
  .command("status", "View live auction state and current price", y => y
    .option("id",     { type: "number", demandOption: true })
    .option("seller", { type: "string", demandOption: true })
  )
  .command("schedule", "Print full price schedule for an auction", y => y
    .option("id",     { type: "number", demandOption: true })
    .option("seller", { type: "string", demandOption: true })
  )
  .demandCommand(1)
  .help()
  .parseSync();

const cmd = argv._[0] as string;

(async () => {
  try {
    switch (cmd) {

      case "create": {
        const { program, keypair, conn } = await setup(argv.wallet as string);
        const id = new BN(argv.id as number);
        const currentSlotNum = await conn.getSlot("confirmed");
        const startSlot = (argv["start-slot"] as number) || currentSlotNum;

        const startPrice = Math.floor((argv["start-price"] as number) * LAMPORTS_PER_SOL);
        const floorPrice = Math.floor((argv["floor-price"] as number) * LAMPORTS_PER_SOL);
        const decaySlots = argv["decay-slots"] as number;
        const priceSteps = argv["price-steps"] as number;

        const [auctionPk] = auctionPDA(id, keypair.publicKey);
        const [vaultPk]   = vaultPDA(id);
        const spinner = ora("Creating auction...").start();

        const tx = await program.methods
          .createAuction(id, new BN(startPrice), new BN(floorPrice),
            new BN(startSlot), new BN(decaySlots), new BN(priceSteps),
            argv.title as string)
          .accounts({ seller: keypair.publicKey, auction: auctionPk, vault: vaultPk, systemProgram: SystemProgram.programId })
          .rpc();

        spinner.succeed(chalk.green("Auction created!"));
        console.log(chalk.bold("  Auction PDA:"), chalk.cyan(auctionPk.toBase58()));
        console.log(chalk.bold("  TX:"), chalk.yellow(`https://explorer.solana.com/tx/${tx}?cluster=devnet`));
        console.log(chalk.bold("  Seller ID for bids:"), keypair.publicKey.toBase58());
        break;
      }

      case "bid": {
        const { program, keypair, conn } = await setup(argv.wallet as string);
        const id     = new BN(argv.id as number);
        const seller = new PublicKey(argv.seller as string);
        const amount = Math.floor((argv.amount as number) * LAMPORTS_PER_SOL);

        const [auctionPk]    = auctionPDA(id, seller);
        const [vaultPk]      = vaultPDA(id);
        const [settlementPk] = settlementPDA(id);

        // Show current price before sending
        const a           = await program.account.auctionState.fetch(auctionPk);
        const currentSlot = await conn.getSlot("confirmed");
        const currentPrice = computePrice(a, currentSlot);
        console.log(chalk.bold(`\n  Current price: ${currentPrice / LAMPORTS_PER_SOL} SOL`));
        console.log(chalk.bold(`  Your bid:      ${amount / LAMPORTS_PER_SOL} SOL`));

        if (amount < currentPrice) {
          console.error(chalk.red(`\n  ✗ Bid too low — current price is ${currentPrice / LAMPORTS_PER_SOL} SOL`));
          process.exit(1);
        }

        const spinner = ora("Placing bid...").start();
        const tx = await program.methods
          .placeBid(new BN(amount))
          .accounts({ bidder: keypair.publicKey, seller, auction: auctionPk, vault: vaultPk, settlement: settlementPk, systemProgram: SystemProgram.programId })
          .rpc();

        spinner.succeed(chalk.green("🎉 Bid won! Auction settled."));
        const settled = await program.account.settlementRecord.fetch(settlementPk);
        console.log(chalk.bold("  Price paid:  "), settled.pricePaid.toNumber() / LAMPORTS_PER_SOL, "SOL");
        console.log(chalk.bold("  Overpayment: "), settled.overpayment.toNumber() / LAMPORTS_PER_SOL, "SOL (refunded)");
        console.log(chalk.bold("  TX:"), chalk.yellow(`https://explorer.solana.com/tx/${tx}?cluster=devnet`));
        break;
      }

      case "cancel": {
        const { program, keypair } = await setup(argv.wallet as string);
        const id = new BN(argv.id as number);
        const [auctionPk] = auctionPDA(id, keypair.publicKey);
        const spinner = ora("Cancelling...").start();
        const tx = await program.methods.cancelAuction()
          .accounts({ seller: keypair.publicKey, auction: auctionPk })
          .rpc();
        spinner.succeed("Cancelled.");
        console.log(chalk.yellow(`https://explorer.solana.com/tx/${tx}?cluster=devnet`));
        break;
      }

      case "status": {
        const conn    = new Connection(clusterApiUrl("devnet"), "confirmed");
        const provider = new AnchorProvider(conn, null as any, {});
        const program  = new anchor.Program(IDL, PROGRAM_ID, provider) as any;

        const id     = new BN(argv.id as number);
        const seller = new PublicKey(argv.seller as string);
        const [auctionPk] = auctionPDA(id, seller);

        const spinner = ora("Fetching...").start();
        const a       = await program.account.auctionState.fetch(auctionPk);
        const slot    = await conn.getSlot("confirmed");
        const price   = computePrice(a, slot);
        spinner.stop();

        printAuction(a, auctionPk, price, slot);
        break;
      }

      case "schedule": {
        const conn     = new Connection(clusterApiUrl("devnet"), "confirmed");
        const provider = new AnchorProvider(conn, null as any, {});
        const program  = new anchor.Program(IDL, PROGRAM_ID, provider) as any;

        const id     = new BN(argv.id as number);
        const seller = new PublicKey(argv.seller as string);
        const [auctionPk] = auctionPDA(id, seller);

        const a = await program.account.auctionState.fetch(auctionPk);
        const startSlot = a.startSlot.toNumber();

        console.log(chalk.bold("\n  Price Schedule\n  ─────────────────────────────────"));
        console.log(chalk.dim("  Step  Slot        Price (SOL)"));
        console.log(chalk.dim("  ───   ─────────   ───────────"));

        for (let step = 0; step <= a.priceSteps.toNumber(); step++) {
          const slot  = startSlot + step * a.decaySlots.toNumber();
          const price = computePrice(a, slot);
          const bar   = "█".repeat(Math.round(10 * (price - a.floorPrice.toNumber()) / (a.startPrice.toNumber() - a.floorPrice.toNumber())));
          const isCurrent = step === a.priceSteps.toNumber() ? " ← floor" : "";
          console.log(`  ${String(step).padStart(4)}  ${String(slot).padEnd(10)}  ${(price / LAMPORTS_PER_SOL).toFixed(4)} SOL  ${bar}${isCurrent}`);
        }
        console.log();
        break;
      }
    }
  } catch (e: any) {
    console.error(chalk.red("\n✗"), e.message ?? e);
    if (e.logs) e.logs.forEach((l: string) => console.error(chalk.dim(" ", l)));
    process.exit(1);
  }
})();
