/**
 * High-level flows — the app-facing surface that wraps the raw kassandra-market
 * + MetaDAO builders into whole user actions:
 *   • {@link composeMarketInstructions} / {@link activateInstruction} — the keeper
 *     "compose → activate" market bring-up.
 *   • {@link buyInstructions} / {@link sellInstructions} — take / close a net
 *     YES/NO position without touching conditional tokens directly.
 *   • {@link redeemInstructions} — claim the resolved payout.
 *   • {@link collectFeeInstruction} — crank the protocol fee to the futarchy.
 *   • {@link closeMarketInstruction} — reclaim a settled market's rent to the creator.
 *   • {@link buildJupiterEntryRequest} / {@link composeWithEntry} — the thin,
 *     offline Jupiter any-token-entry boundary.
 */
export * from "./compose.js";
export * from "./createAll.js";
export * from "./trade.js";
export * from "./redeem.js";
export * from "./collectFee.js";
export * from "./closeMarket.js";
export * from "./jupiter.js";
export * from "./atas.js";
export { toAddr } from "./util.js";
