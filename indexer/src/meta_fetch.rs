//! Autonomous fetch of externally-hosted oracle metadata JSON.
//!
//! `write_oracle_meta` commits a `uri` + `uri_hash` on-chain; the extended JSON it
//! points at may be hosted ANYWHERE — our app host OR a third-party service. This
//! reconciler makes indexing host-agnostic: it periodically finds oracles whose
//! committed JSON we don't have yet, GETs the `uri`, verifies `sha256(body) ==
//! uri_hash`, and stores it in `oracle_meta_json`. Because the served JSON is gated
//! on that same hash (see `api::get_oracle_meta_json`), a fetched copy is only ever
//! stored — and later served — when it matches the on-chain commitment.
//!
//! # SSRF hardening
//!
//! The `uri` is attacker-controllable (anyone can create an oracle with any URL), so
//! every fetch is guarded: http/https only, the host must resolve to a GLOBAL
//! address (no loopback / private / link-local / unique-local / CGNAT targets),
//! redirects are disabled (no internal pivot), and the body is size- + time-capped.
//! The fetched bytes are never reflected back to a caller — a mismatching body is
//! dropped — so this is at worst blind, and the IP guard blocks even that.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio_postgres::Client;

use crate::db;

/// Reject a fetched body larger than this (the metadata JSON is small).
const MAX_META_BYTES: usize = 256 * 1024;
/// Per-fetch timeout (connect + read).
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Oracles (re)tried per reconcile tick.
const BATCH: i64 = 50;

/// Periodically fetch + verify + store externally-hosted metadata JSON. Runs for
/// the life of the process; each tick is best-effort and self-retrying.
pub async fn reconcile_loop(client: Arc<Client>, interval_ms: u64) {
    let http = match reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("kassandra-indexer/metadata-fetch")
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::error!("[meta] http client build failed; fetcher disabled: {e}");
            return;
        }
    };
    log::info!("[meta] metadata fetcher started (every {interval_ms}ms)");
    loop {
        tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        match reconcile_once(&http, &client).await {
            Ok(n) if n > 0 => log::info!("[meta] indexed {n} externally-hosted metadata json"),
            Ok(_) => {}
            Err(e) => log::warn!("[meta] reconcile query failed: {e}"),
        }
    }
}

async fn reconcile_once(http: &reqwest::Client, client: &Client) -> anyhow::Result<usize> {
    let todo = db::oracles_missing_meta_json(client, BATCH).await?;
    let mut stored = 0usize;
    for (oracle, uri, uri_hash) in todo {
        if let Some((json, sha)) = fetch_and_verify(http, &uri, &uri_hash).await {
            match db::upsert_oracle_meta_json(client, &oracle, &json, &sha).await {
                Ok(()) => stored += 1,
                Err(e) => log::warn!("[meta] store {oracle}: {e}"),
            }
        }
    }
    Ok(stored)
}

/// GET `uri` under the SSRF guards and return `(body, sha256_hex)` ONLY if
/// `sha256(body) == uri_hash`. `None` on any failure (kept in the work list, so it
/// retries next tick).
async fn fetch_and_verify(
    http: &reqwest::Client,
    uri: &str,
    uri_hash: &str,
) -> Option<(String, String)> {
    if !is_fetchable(uri).await {
        return None;
    }
    let resp = http.get(uri).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    if resp
        .content_length()
        .is_some_and(|l| l as usize > MAX_META_BYTES)
    {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    if bytes.len() > MAX_META_BYTES {
        return None;
    }
    let sha = sha256_hex(&bytes);
    if sha != uri_hash {
        log::debug!("[meta] {uri}: sha256 {sha} != committed {uri_hash}; skipping");
        return None;
    }
    let json = String::from_utf8(bytes.to_vec()).ok()?;
    Some((json, sha))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Scheme + resolved-IP guard: http/https only, and EVERY resolved address must be
/// global (reject if any is internal — no partial-trust of multi-homed hosts).
async fn is_fetchable(uri: &str) -> bool {
    let url = match reqwest::Url::parse(uri) {
        Ok(u) => u,
        Err(_) => return false,
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str().map(str::to_owned) else {
        return false;
    };
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<_> = match tokio::net::lookup_host((host, port)).await {
        Ok(it) => it.collect(),
        Err(_) => return false,
    };
    // Reject hosts that resolve to nothing, or to ANY internal address.
    !addrs.is_empty() && addrs.iter().all(|a| !is_disallowed(a.ip()))
}

/// Whether an IP is off-limits for the fetcher (internal / special-use ranges).
/// Built from stable `std::net` predicates since `IpAddr::is_global` is unstable.
fn is_disallowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10 CGNAT
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique-local fc00::/7
                || v6.to_ipv4_mapped().is_some_and(|m| is_disallowed(IpAddr::V4(m)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn disallows_internal_targets() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.5",
            "172.16.9.9",
            "169.254.169.254", // cloud metadata
            "100.64.1.1",      // CGNAT
            "0.0.0.0",
        ] {
            assert!(
                is_disallowed(ip.parse::<Ipv4Addr>().unwrap().into()),
                "{ip} should be disallowed"
            );
        }
        assert!(is_disallowed(Ipv6Addr::LOCALHOST.into()));
        assert!(is_disallowed("fe80::1".parse::<Ipv6Addr>().unwrap().into()));
        assert!(is_disallowed("fc00::1".parse::<Ipv6Addr>().unwrap().into()));
        // ::ffff:127.0.0.1 (v4-mapped loopback)
        assert!(is_disallowed(
            "::ffff:127.0.0.1".parse::<Ipv6Addr>().unwrap().into()
        ));
    }

    #[test]
    fn allows_public_targets() {
        for ip in ["1.1.1.1", "8.8.8.8", "93.184.216.34"] {
            assert!(
                !is_disallowed(ip.parse::<Ipv4Addr>().unwrap().into()),
                "{ip} should be allowed"
            );
        }
        assert!(!is_disallowed(
            "2606:4700:4700::1111".parse::<Ipv6Addr>().unwrap().into()
        ));
    }

    #[tokio::test]
    async fn rejects_non_http_schemes() {
        // The scheme check short-circuits before any network lookup.
        for uri in [
            "file:///etc/passwd",
            "ftp://x/y",
            "gopher://x",
            "data:text/plain,hi",
        ] {
            assert!(!is_fetchable(uri).await, "{uri} should be rejected");
        }
    }
}
