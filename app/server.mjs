/**
 * Production server for the Kassandra dApp.
 *
 * Serves the built Vite SPA from `dist/` AND proxies `/indexer/*` to the indexer
 * backend over Render's PRIVATE network. The indexer is a private service with no
 * public URL — only this app server can reach it (at `INDEXER_URL`, injected from
 * the private service's internal host:port). The browser therefore calls the
 * app's OWN origin (`/indexer/...`), never the indexer directly.
 *
 * Dependency-free (Node built-ins only): static file serving with an SPA
 * fallback + a streaming reverse proxy.
 */
import { createReadStream } from 'node:fs'
import { stat } from 'node:fs/promises'
import { createServer, request as httpRequest } from 'node:http'
import { dirname, extname, join, normalize } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = dirname(fileURLToPath(import.meta.url))
const DIST = join(ROOT, 'dist')
const PORT = Number(process.env.PORT ?? 3000)
const PROXY_PREFIX = '/indexer'

// The private indexer's internal base URL. Render injects the private service's
// INDEXER_HOST + INDEXER_PORT (or INDEXER_HOSTPORT); INDEXER_URL is an explicit
// override. Empty → the proxy replies 503 (feature effectively off).
function resolveIndexerUrl() {
  if (process.env.INDEXER_URL) return process.env.INDEXER_URL
  if (process.env.INDEXER_HOSTPORT) return `http://${process.env.INDEXER_HOSTPORT}`
  if (process.env.INDEXER_HOST) {
    return `http://${process.env.INDEXER_HOST}:${process.env.INDEXER_PORT ?? 10000}`
  }
  return ''
}
const INDEXER_URL = resolveIndexerUrl()

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.webp': 'image/webp',
  '.ico': 'image/x-icon',
  '.woff2': 'font/woff2',
  '.woff': 'font/woff',
  '.map': 'application/json; charset=utf-8',
  '.txt': 'text/plain; charset=utf-8',
  '.wasm': 'application/wasm',
}

/**
 * Reverse-proxy a request to the private indexer at `upstreamPath`. Both the
 * oracle routes (reached under `/indexer/*`, prefix stripped → `/rpc`,`/events`…)
 * and the market routes (reached under `/api/*`, passed through unchanged — the
 * indexer serves them AT `/api/*`) hit the SAME single indexer.
 */
function proxyToIndexer(req, res, upstreamPath) {
  if (!INDEXER_URL) {
    res.writeHead(503, { 'content-type': 'application/json' })
    res.end(JSON.stringify({ error: 'indexer not configured' }))
    return
  }
  const target = new URL(INDEXER_URL)
  const upstream = httpRequest(
    {
      protocol: target.protocol,
      hostname: target.hostname,
      port: target.port || 80,
      method: req.method,
      path: upstreamPath,
      headers: { ...req.headers, host: target.host },
      timeout: 15_000,
    },
    (up) => {
      res.writeHead(up.statusCode ?? 502, up.headers)
      up.pipe(res)
    },
  )
  upstream.on('error', () => {
    if (!res.headersSent) res.writeHead(502, { 'content-type': 'application/json' })
    res.end(JSON.stringify({ error: 'indexer unreachable' }))
  })
  upstream.on('timeout', () => upstream.destroy())
  req.pipe(upstream)
}

/** Resolve a request path to a safe file inside DIST, or null if it escapes. */
function safePath(urlPath) {
  const clean = normalize(decodeURIComponent(urlPath.split('?')[0])).replace(/^(\.\.[/\\])+/, '')
  const full = join(DIST, clean)
  return full.startsWith(DIST) ? full : null
}

async function serveFile(res, file, { immutable = false } = {}) {
  const type = MIME[extname(file).toLowerCase()] ?? 'application/octet-stream'
  const headers = { 'content-type': type }
  if (immutable) headers['cache-control'] = 'public, max-age=31536000, immutable'
  res.writeHead(200, headers)
  createReadStream(file).pipe(res)
}

const server = createServer((req, res) => {
  const url = req.url ?? '/'

  // 1) Proxy the indexer API over the private network. Oracle routes live under
  //    `/indexer/*` (prefix stripped); market routes live under `/api/*` (passed
  //    through as-is). Both target the same single indexer.
  if (url === PROXY_PREFIX || url.startsWith(`${PROXY_PREFIX}/`)) {
    proxyToIndexer(req, res, url.slice(PROXY_PREFIX.length) || '/')
    return
  }
  // Public oracle-metadata JSON host: the on-chain `uri` points here. Map it to
  // the indexer's private `/oracles/{pk}/meta-json` route (GET serves it gated by
  // uri_hash; POST stores the app-supplied JSON). Must precede the generic /api/*.
  const metaMatch = url.match(/^\/api\/oracle\/([^/?]+)\/metadata\.json(?:\?.*)?$/)
  if (metaMatch) {
    proxyToIndexer(req, res, `/oracles/${metaMatch[1]}/meta-json`)
    return
  }
  if (url === '/api' || url.startsWith('/api/')) {
    proxyToIndexer(req, res, url)
    return
  }

  // 2) Static assets (fingerprinted → immutable) or any existing file.
  void (async () => {
    const file = safePath(url === '/' ? '/index.html' : url)
    if (file) {
      try {
        const s = await stat(file)
        if (s.isFile()) {
          await serveFile(res, file, { immutable: url.startsWith('/assets/') })
          return
        }
      } catch {
        /* fall through to SPA index */
      }
    }
    // 3) SPA fallback: React Router owns client-side routes.
    await serveFile(res, join(DIST, 'index.html'))
  })()
})

server.listen(PORT, () => {
  // eslint-disable-next-line no-console
  console.log(
    `[app] serving ${DIST} on :${PORT}; indexer proxy → ${INDEXER_URL || '(unconfigured)'}`,
  )
})
