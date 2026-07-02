# Kassandra documentation site

The [Kassandra](https://github.com/Dodecahedr0x/kassandra) documentation, built with
[Mintlify](https://mintlify.com). Content lives in this directory as MDX; the site
configuration is `docs.json`.

## Develop locally

```bash
# from docs-site/
npm install            # installs the `mint` CLI locally
npm run dev            # serves the docs at http://localhost:3000

# or, without installing:
npx mint dev
```

## Validate

```bash
npm run check-links    # runs `mint broken-links`
```

The same check runs in CI on every push to `master` and on pull requests
(`.github/workflows/docs.yml`).

## How publishing works

Publishing is performed **entirely by GitHub Actions** (`.github/workflows/docs.yml`). On
every push to `master` the workflow validates (`mint validate` + `mint broken-links`),
builds a static site (`mint export`), rewrites the export's root-absolute URLs to the
GitHub Pages base path (`scripts/rewrite-base-path.mjs`), and deploys to **GitHub Pages**.
Pull requests build + validate only; they do not deploy.

### One-time setup (done once by a maintainer)

1. In the repo, go to **Settings → Pages → Build and deployment** and set **Source** to
   **GitHub Actions**.
2. Push to `master` (or run **Actions → Docs → Run workflow**). The live URL appears on the
   workflow's `github-pages` environment.
3. *(Optional)* For a custom domain, add a `CNAME` file to this directory with your domain
   and set it under **Settings → Pages**. The workflow then serves from the domain root and
   skips the base-path rewrite.

> Mintlify's static export uses root-absolute paths; served from a project sub-path they'd
> 404, so the workflow rewrites them. Direct page loads, assets, and refreshes resolve.
> A custom domain (domain-root serving) gives pixel-perfect client-side navigation.

**Alternative:** to use Mintlify's hosted platform instead, connect the repo at
<https://dashboard.mintlify.com> (content dir `docs-site`, branch `master`, install the
GitHub App) and reduce the workflow to just the validation steps. See the
**Operations → This documentation site** page for details.

## Structure

- `docs.json` — site config: theme, colors, navigation (tabs → groups → pages).
- `guide/`, `concepts/`, `architecture/`, `app/`, `ops/`, `contributing/` — the
  Documentation tab.
- `protocol/`, `challenge/`, `sdk/` — the Reference tab.
