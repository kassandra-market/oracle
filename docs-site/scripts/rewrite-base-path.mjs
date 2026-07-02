// Rewrite root-absolute references in a `mint export` static build so it can be served
// from a sub-path (e.g. GitHub Pages project sites at https://user.github.io/<repo>/).
//
// Mintlify's static export emits root-absolute URLs (`/_next/...`, `/guide/...`). Served
// at a sub-path those 404. This prefixes every absolute asset/route reference with the
// base path. Pass an EMPTY base path (custom domain / root serving) to make it a no-op.
//
// Usage: node rewrite-base-path.mjs <exportDir> <basePath>
//   node rewrite-base-path.mjs ./_site /kassandra
//   node rewrite-base-path.mjs ./_site ""            # root serving, no rewrite

import { readdirSync, readFileSync, writeFileSync, statSync } from "node:fs";
import { join, extname } from "node:path";

const [dir, rawBase = ""] = process.argv.slice(2);
if (!dir) {
  console.error("usage: node rewrite-base-path.mjs <exportDir> <basePath>");
  process.exit(1);
}

// Normalize: no trailing slash, leading slash required when non-empty.
let base = rawBase.trim().replace(/\/+$/, "");
if (base && !base.startsWith("/")) base = "/" + base;

if (!base) {
  console.log("base path is empty — serving from root, no rewrite needed.");
  process.exit(0);
}

const exts = new Set([".html", ".js", ".css", ".json", ".txt", ".xml", ".map"]);
let filesChanged = 0;

function walk(d) {
  for (const entry of readdirSync(d)) {
    const p = join(d, entry);
    const st = statSync(p);
    if (st.isDirectory()) walk(p);
    else if (exts.has(extname(p))) rewrite(p);
  }
}

function rewrite(file) {
  const src = readFileSync(file, "utf8");
  let out = src;
  // Attribute values: href="/…", src="/…", content="/…", poster="/…" (and single quotes).
  out = out.replace(/(\b(?:href|src|content|poster|data-href)=)(["'])\/(?!\/)/g, `$1$2${base}/`);
  // srcset="/a 1x, /b 2x"
  out = out.replace(/(\bsrcset=)(["'])([^"']*)\2/g, (m, attr, q, val) => {
    const fixed = val.replace(/(^|,\s*)\/(?!\/)/g, `$1${base}/`);
    return `${attr}${q}${fixed}${q}`;
  });
  // CSS url(/…) with optional quotes.
  out = out.replace(/url\((["']?)\/(?!\/)/g, `url($1${base}/`);
  // JS/JSON string literals that point at the export's own assets/routes.
  out = out.replace(/(["'`])\/_next\//g, `$1${base}/_next/`);
  out = out.replace(/(["'`])\/(favicons?|images|_mintlify|icons)\//g, `$1${base}/$2/`);
  if (out !== src) {
    writeFileSync(file, out);
    filesChanged++;
  }
}

walk(dir);
console.log(`rewrote absolute references to base "${base}" in ${filesChanged} files.`);
