# Hahakigami website

Static SvelteKit site (Svelte 5, `adapter-static`, fully prerendered). It has its own pnpm root, separate from the app.

```sh
cd site
pnpm install
pnpm dev        # http://localhost:4620
pnpm check      # svelte-check
pnpm build      # writes site/build
pnpm preview
```

## Before launch

Edit `src/lib/config.ts`:

- `DOWNLOAD_URL`: `null` renders the dashed "coming soon" Download button; set the signed build's URL to turn it into a solid red link.
- `SITE_URL`: absolute origin (for example `https://example.com`) so the Open Graph image URL is absolute.

## Deploy to Cloudflare Pages

- Root directory: `site`
- Build command: `pnpm build`
- Build output directory: `build`
