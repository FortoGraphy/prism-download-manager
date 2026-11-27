# Search engines and prismdl.site

How the site is set up for search, and what to run when it changes. `site/tools/` is not
published (pages.yml deletes it before deploying).

## What is on the site

- `index.html` plus landing pages built by `build-pages.py` from `tools/pages/*.html`:
  `/windows/`, `/mac/`, `/linux/`, `/browser-extension/`, `/idm-alternative/`, `/hugging-face/`,
  and `404.html` (noindex). The pages target real queries from Google's autocomplete
  (download manager for pc, download manager chrome extension, idm alternative for mac,
  hugging face download slow, download gguf from huggingface, ...).
- Guides and comparisons (`ARTICLES` in `build-pages.py`, sources in `tools/pages/guides/*.html` and
  `tools/pages/best-download-manager.html`, hub `/guides/`): how-to articles that answer real queries
  (download faster on pc, chrome download slow fix, resume failed download, download large files,
  how to download magnet links, make torrents download faster, best download manager). Each has a
  visible byline, `published`/`updated` meta (Article JSON-LD), a FAQ and links to the other guides.
  Rules: solve the reader's problem first and say plainly when Prism does *not* help; facts about other
  apps must be checkable and listed under "Sources"; re-check the comparison every few months and bump
  `updated`. No mass-produced pages (Google's scaled-content policy).
- JSON-LD: `WebSite`, `Person`, `SoftwareApplication`, `SoftwareSourceCode` on the home page
  (the landing pages copy them), `WebPage` + `BreadcrumbList` on each landing page, and
  `FAQPage` written from each page's visible FAQ. Google stopped showing FAQ rich results in
  2026, but the markup is still valid and other engines and AI search read it. `VideoObject`
  for the film on the home page.
- `sitemap.xml` (written by `build-pages.py`, `<lastmod>` only moves when a page changes),
  `robots.txt` pointing at it, the IndexNow key `<32 hex>.txt`, and `llms.txt` (a plain summary
  some AI tools read; Google ignores it).
- Download links in page sources use `{{version}}`, so `build-pages.py` keeps them on the release
  that `index.html` links to.

## When something changes

1. Edit `site/index.html` or `site/tools/pages/<page>.html` (never the generated
   `site/<page>/index.html` or the FAQ JSON-LD between the `faq-jsonld` markers).
2. `python site/tools/build-pages.py`
3. After a release, the "Site: x.y.z links" step also bumps `softwareVersion`, `dateModified`
   and `releaseNotes` in the home page JSON-LD; then run `build-pages.py` so the landing pages
   get the new download links.
4. After the site is live: `python site/tools/indexnow.py` (Bing, Yandex, Seznam…), and in
   Google Search Console, *URL inspection → Request indexing* for pages that matter.

## Verifying ownership

Google Search Console (https://search.google.com/search-console), signed in with the owner's
Google account:

- **Domain property (recommended):** enter `prismdl.site`, copy the `google-site-verification=…`
  TXT record into the domain's DNS at the registrar, wait until it resolves, click *Verify*.
  Covers http/https and every subdomain. Nothing changes in this repository.
- **URL-prefix property** `https://prismdl.site/`, if DNS isn't at hand: either the *HTML tag*
  method (put the `<meta name="google-site-verification" content="…">` tag where the comment
  in `index.html`'s `<head>` says) or the *HTML file* method (put `google<token>.html` into
  `site/`). Keep the tag or file for good: removing it un-verifies the site.

Then: *Sitemaps* → add `sitemap.xml`.

Bing Webmaster Tools (https://www.bing.com/webmasters): *Import from Google Search Console*
once Google is verified (no extra tag needed), or add the site with the `msvalidate.01` meta
tag / `BingSiteAuth.xml` file. Submit `https://prismdl.site/sitemap.xml`.
