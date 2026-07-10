/**
 * Assemble a MetaDAO {@link flows.MarketRefs} from an already-Active decoded
 * {@link Market} (pure, NO React).
 *
 * The generic derivation now lives in the SDK ({@link flows.marketRefsFromAccount}
 * — a second consumer such as a keeper reuses it); this thin app wrapper keeps the
 * funding-form call sites' signature (and the typed {@link ValidationError} on a
 * bad market pubkey) unchanged.
 */
import { flows, type Market } from "@kassandra-market/markets";
import { toAddress, type AddressInput } from "./ata";

/**
 * Rebuild the composed {@link flows.MarketRefs} for an Active market from its
 * PDA + decoded account. `marketPubkey` is the `Market` PDA (`useMarketDetail`'s
 * `detail.pubkey`); `market` is the decoded account (carries the composed
 * bindings). Everything else is re-derived deterministically by the SDK.
 */
export function marketRefs(
  marketPubkey: AddressInput,
  market: Market,
): Promise<flows.MarketRefs> {
  return flows.marketRefsFromAccount({
    market: toAddress("Market", marketPubkey),
    decoded: market,
  });
}
