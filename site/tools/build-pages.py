"""Build the website's landing pages, FAQ structured data and sitemap.

Run from the repository root:  python site/tools/build-pages.py

* site/tools/pages/<page>.html -> site/<page>/index.html, wrapped in the shared shell. The icon
  sprite, nav, footer, stylesheets, scripts and download cards are copied from site/index.html,
  so the pages stay in step with it. Run this again after editing site/index.html, and after the
  release step that updates the download links ("Site: x.y.z links").
* FAQPage JSON-LD for site/index.html and every page, written from the visible FAQ
  (the <div data-faq> accordion). Search engines want the markup to match what visitors see:
  edit the FAQ text, never the JSON.
* site/sitemap.xml with every page; a page's <lastmod> changes only when the page changes
  (dates are kept in site/tools/pages/lastmod.json).

Page source format: a comment block with "key: value" lines, then the content of <main>.
  title, description, crumb   required (crumb: the page's name in the breadcrumb)
  og_title                    optional, for link previews
  noindex                     optional, "yes": not in the sitemap, no canonical (the 404 page)
  published, updated          articles (guides, comparisons): YYYY-MM-DD, written into the
                              Article JSON-LD; "updated" also goes into the visible byline
  headline                    optional, the Article headline (defaults to the title)
Placeholders in the content:
  {{version}}                              the release the download links point to (x.y.z)
  {{dl:windows}} {{dl:mac}} {{dl:linux}}   one download card copied from site/index.html
  {{dl:all}}                               all three cards
  {{related}}                              links to the other pages and the guides
  {{guides}}                               links to the guides (without the page itself)
  {{byline}}                               author and "updated" date (articles)
  {{cta}}                                  the closing download call to action
Articles can live under a parent path (guides/<slug>): their source is
site/tools/pages/guides/<slug>.html and their breadcrumb gets the parent as a middle step.
Python 3.8+, standard library only.
"""
import datetime
import hashlib
import html
import json
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
SITE = os.path.join(ROOT, "site")
SRC = os.path.join(SITE, "tools", "pages")
BASE = "https://prismdl.site"

# Landing pages. Order = order in the sitemap and in the "related" links.
PAGES = [
    ("windows", "Prism for Windows", "Installer, MSI or portable for Windows 10 and 11."),
    ("mac", "Prism for Mac", "Apple Silicon and Intel, macOS 11 or later."),
    ("linux", "Prism for Linux", "deb, rpm or AppImage for any distribution."),
    ("browser-extension", "Browser extension", "Chrome, Edge, Brave, Opera, Vivaldi and Firefox."),
    ("idm-alternative", "An alternative to IDM", "What carries over from Internet Download Manager."),
    ("hugging-face", "Hugging Face models", "GGUF and safetensors, only the files you need."),
]
# Guides and comparisons: (path, title, description). "guides/x" -> site/guides/x/index.html,
# written from site/tools/pages/guides/x.html. HUB is the page that lists them all.
ARTICLES = [
    ("best-download-manager", "Best free download managers", "An honest comparison for Windows, Mac and Linux."),
    ("guides/download-faster", "How to download faster on a PC", "Find the bottleneck first, then fix the right thing."),
    ("guides/chrome-downloads-slow", "Chrome downloads slow?", "Why it happens and what actually helps."),
    ("guides/resume-failed-download", "Resume a failed download", "Chrome, Edge and Firefox, and how to stop starting over."),
    ("guides/download-large-files", "Download large files", "Fast, complete and verified, even at 50 GB."),
    ("guides/magnet-links", "Open magnet links", "What a magnet link is and how to download one."),
    ("guides/torrents-faster", "Make torrents download faster", "Seeds, ports and VPNs, and the tips that are myths."),
]
HUB = ("guides", "Guides", "Every guide in one place.")
PARENTS = {"guides": "Guides"}
# Built too, but not linked from the "related" section: (source name, output path under site/)
OTHER = [("404", "404.html")]


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def write_if_changed(path, text):
    old = read(path) if os.path.exists(path) else None
    if old == text:
        return False
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    return True


def grab(pattern, text, what):
    m = re.search(pattern, text, re.S)
    if not m:
        sys.exit(f"build-pages: {what} not found in site/index.html")
    return m.group(0)


def rooted(fragment):
    """Make same-page links (#features) and relative asset paths work from /<page>/."""
    fragment = re.sub(r'(<a\b[^>]*?\bhref=")#', r"\1/#", fragment)
    fragment = fragment.replace('href="/#top"', 'href="/"')
    return re.sub(r'\b(src|srcset|href|data-src|data-srcset)="(assets/)', r'\1="/\2', fragment)


# --------------------------------------------------------------------------- shared shell
index = read(os.path.join(SITE, "index.html"))
SPRITE = grab(r'<svg width="0" height="0"[^>]*>.*?</svg>', index, "icon sprite")
NAV = rooted(grab(r'<div class="nav-wrap">.*?</nav>\s*</div>', index, "nav"))
FOOTER = rooted(grab(r'<footer class="footer">.*?</footer>', index, "footer"))
THEME_JS = grab(r"<script>\s*/\* Theme before first paint.*?</script>", index, "theme script")
STYLES = rooted("\n".join(re.findall(r'<link rel="stylesheet" href="assets/[^"]+">', index)))
SCRIPT = rooted(grab(r'<script src="assets/js/site\.js[^"]*" defer></script>', index, "site.js tag"))
CARDS = {}  # data-os -> download card, without its link to the page it would be shown on
for m in re.finditer(r'(?s)<article class="dl-card[^"]*" data-os="(\w+)">.*?</article>', index):
    card = re.sub(r'(?s)\s*<p class="dl-more">.*?</p>', "", m.group(0))
    CARDS[m.group(1)] = rooted(re.sub(r" reveal d\d\b", " reveal", card))
for key in ("windows", "mac", "linux"):
    if key not in CARDS:
        sys.exit(f"build-pages: download card {key!r} not found in site/index.html")
VERSION = re.search(r"/releases/download/v(\d+\.\d+\.\d+)/", CARDS["windows"]).group(1)

ld_site = json.loads(re.search(r'<script type="application/ld\+json" id="ld-site">(.*?)</script>', index, re.S).group(1))
GRAPH = {node["@type"]: node for node in ld_site["@graph"]}


# --------------------------------------------------------------------------- FAQ -> JSON-LD
def plain(fragment):
    fragment = re.sub(r"(?s)<svg\b.*?</svg>|<button\b.*?</button>", "", fragment)
    fragment = re.sub(r"</p>\s*<p>|<br\s*/?>|</pre>", " ", fragment)
    text = html.unescape(re.sub(r"<[^>]+>", "", fragment))
    return re.sub(r"\s+", " ", text).strip()


def faq_items(page_html):
    start = page_html.find("data-faq")
    if start < 0:
        return []
    end = page_html.find("</section>", start)
    block = page_html[start:end if end > 0 else None]
    items = []
    for m in re.finditer(r'(?s)<details class="acc"[^>]*>\s*<summary>(.*?)</summary>\s*<div class="acc-body">(.*?)</div></details>', block):
        items.append({
            "@type": "Question",
            "name": plain(m.group(1)),
            "acceptedAnswer": {"@type": "Answer", "text": plain(m.group(2))},
        })
    return items


def faq_node(page_url, items):
    return {"@type": "FAQPage", "@id": page_url + "#faq", "url": page_url, "mainEntity": items}


def ld_script(data):
    return '<script type="application/ld+json">\n' + json.dumps(data, ensure_ascii=False, indent=2) + "\n</script>"


# --------------------------------------------------------------------------- pages
def parse_source(text):
    m = re.match(r"\s*<!--(.*?)-->\s*", text, re.S)
    if not m:
        sys.exit("build-pages: a page source must start with a <!-- key: value --> block")
    meta = {}
    for line in m.group(1).strip().splitlines():
        if ":" in line:
            k, v = line.split(":", 1)
            meta[k.strip()] = v.strip()
    for k in ("title", "description", "crumb"):
        if not meta.get(k):
            sys.exit(f"build-pages: page meta is missing {k!r}")
    return meta, text[m.end():]


def link_cards(entries, skip):
    return "\n".join(
        f'      <a class="lp-link" href="/{s}/"><b>{html.escape(t)}</b><span>{html.escape(d)}</span></a>'
        for s, t, d in entries if s != skip
    )


def guides(slug):
    return f"""<div class="lp-links">
{link_cards(ARTICLES, slug)}
    </div>"""


def related(slug):
    links = link_cards(PAGES, slug)
    guide_block = "" if slug == HUB[0] else f"""
    <h3 class="h3 lp-links-head reveal">Guides</h3>
    <div class="lp-links reveal">
{link_cards(ARTICLES, slug)}
    </div>"""
    return f"""<section class="section" aria-labelledby="related-title" style="padding-top:0">
  <div class="container">
    <div class="section-head reveal">
      <p class="eyebrow">More about Prism</p>
      <h2 class="h2" id="related-title">Everything else Prism does.</h2>
      <p class="lede">The <a href="/">Prism home page</a> has the full feature tour, benchmarks and the install guide.</p>
    </div>
    <div class="lp-links reveal">
      <a class="lp-link" href="/"><b>Prism home</b><span>Features, benchmarks, film and guide.</span></a>
{links}
    </div>{guide_block}
  </div>
</section>"""


CTA = """<section class="section cta-final" aria-labelledby="cta-title" style="padding-top:0">
  <div class="spectrum-line" aria-hidden="true"></div>
  <div class="container">
    <img class="cta-mark reveal" src="/assets/icons/icon-192.png" width="192" height="192" alt="" loading="lazy" decoding="async">
    <h2 class="h2 reveal" id="cta-title">One beam. <span class="grad-text">Many rays.</span></h2>
    <p class="lede reveal">Free, open source and ready for your next big download.</p>
    <div class="cta-row reveal">
      <a class="btn btn-primary btn-xl" href="/#download" data-dl-primary>
        <svg class="i" aria-hidden="true"><use href="#i-download"/></svg>
        <span data-dl-label>Download Prism</span>
      </a>
      <a class="btn btn-glass btn-xl" href="https://github.com/FortoGraphy/prism-download-manager" data-gh-star>
        <svg class="i" aria-hidden="true"><use href="#i-star"/></svg> Star on GitHub <span class="count" data-stars hidden></span>
      </a>
    </div>
  </div>
</section>"""


def build_page(path, meta, body):
    noindex = meta.get("noindex") == "yes"
    url = f"{BASE}/{path}" if path else BASE + "/"
    body = body.replace("{{version}}", VERSION)
    for key in ("windows", "mac", "linux"):
        body = body.replace("{{dl:%s}}" % key, CARDS[key])
    body = body.replace("{{dl:all}}", '<div class="dl-grid">\n' + "\n".join(CARDS[k] for k in ("windows", "mac", "linux")) + "\n</div>")
    slug = path.strip("/")
    article = bool(meta.get("published"))
    byline = ""
    if article:
        day = datetime.date.fromisoformat(meta.get("updated") or meta["published"])
        byline = (f'<p class="byline">By <a href="{GRAPH["Person"]["url"]}">{html.escape(GRAPH["Person"]["name"])}</a>, '
                  f'the developer of Prism · Updated <time datetime="{day.isoformat()}">{day.day} {day:%B %Y}</time></p>')
    body = body.replace("{{related}}", related(slug)).replace("{{cta}}", CTA)
    body = body.replace("{{guides}}", guides(slug)).replace("{{byline}}", byline)
    left = re.findall(r"\{\{[^}]*\}\}", body)
    if left:
        sys.exit(f"build-pages: {path}: unknown placeholder(s) {left}")

    title, desc = meta["title"], meta["description"]
    og_title = meta.get("og_title", title)
    crumbs = [("Prism", BASE + "/")]
    if "/" in slug:
        parent = slug.split("/")[0]
        crumbs.append((PARENTS[parent], f"{BASE}/{parent}/"))
    crumbs.append((meta["crumb"], url))
    webpage = {
        "@type": "WebPage",
        "@id": url + "#webpage",
        "url": url,
        "name": title,
        "description": desc,
        "inLanguage": "en",
        "isPartOf": {"@id": GRAPH["WebSite"]["@id"]},
        "breadcrumb": {"@id": url + "#breadcrumb"},
    }
    # A product page is about Prism; a guide is about its topic and only mentions Prism.
    webpage["mentions" if article or slug == HUB[0] else "about"] = {"@id": GRAPH["SoftwareApplication"]["@id"]}
    graph = [
        webpage,
        {
            "@type": "BreadcrumbList",
            "@id": url + "#breadcrumb",
            "itemListElement": [
                {"@type": "ListItem", "position": i, "name": n, "item": u}
                for i, (n, u) in enumerate(crumbs, 1)
            ],
        },
        GRAPH["WebSite"],
        GRAPH["Person"],
        GRAPH["SoftwareApplication"],
    ]
    if article:
        graph.insert(1, {
            "@type": "Article",
            "@id": url + "#article",
            "headline": meta.get("headline", title),
            "description": desc,
            "inLanguage": "en",
            "datePublished": meta["published"],
            "dateModified": meta.get("updated") or meta["published"],
            "author": {"@id": GRAPH["Person"]["@id"]},
            "publisher": {"@id": GRAPH["Person"]["@id"]},
            "image": f"{BASE}/og.png",
            "mainEntityOfPage": {"@id": url + "#webpage"},
            "isPartOf": {"@id": GRAPH["WebSite"]["@id"]},
        })
    items = faq_items(body)
    if items:
        graph.append(faq_node(url, items))
    e = html.escape
    head_seo = '<meta name="robots" content="noindex">' if noindex else f'<link rel="canonical" href="{url}">'
    page = f"""<!doctype html>
<!-- Written by site/tools/build-pages.py from site/tools/pages/{meta["_src"]}. Edit that file, then run the script. -->
<html lang="en" data-theme="dark" data-accent="prism">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>{e(title)}</title>
<meta name="description" content="{e(desc)}">
{head_seo}
<meta name="theme-color" content="#07080b" media="(prefers-color-scheme: dark)">
<meta name="theme-color" content="#f4f5f8" media="(prefers-color-scheme: light)">
<meta name="color-scheme" content="dark light">

<meta property="og:type" content="{"article" if article else "website"}">
<meta property="og:site_name" content="Prism Download Manager">
<meta property="og:locale" content="en_US">
<meta property="og:title" content="{e(og_title)}">
<meta property="og:description" content="{e(desc)}">
<meta property="og:url" content="{url}">
<meta property="og:image" content="{BASE}/og.png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="Prism download manager — One beam. Many rays.">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="{e(og_title)}">
<meta name="twitter:description" content="{e(desc)}">
<meta name="twitter:image" content="{BASE}/og.png">

<link rel="icon" href="/favicon.ico" sizes="48x48">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="icon" href="/favicon-32.png" type="image/png" sizes="32x32">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="manifest" href="/site.webmanifest">

<link rel="preload" href="/assets/fonts/InterVariable-latin.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preconnect" href="https://api.github.com" crossorigin>
{THEME_JS}
{STYLES}
{SCRIPT}
{ld_script({"@context": "https://schema.org", "@graph": graph})}
</head>
<body>
<a class="skip-link" href="#main">Skip to content</a>

{SPRITE}

{NAV}

<main id="main">
{body.strip()}
</main>

{FOOTER}
</body>
</html>
"""
    return page, noindex


def main():
    lastmod_path = os.path.join(SRC, "lastmod.json")
    lastmod = json.loads(read(lastmod_path)) if os.path.exists(lastmod_path) else {}
    today = datetime.date.today().isoformat()

    def stamp(url, text):
        h = hashlib.sha256(text.encode("utf-8")).hexdigest()[:16]
        prev = lastmod.get(url)
        if not prev or prev[0] != h:
            lastmod[url] = [h, today]
        return lastmod[url][1]

    # The home page: refresh its FAQ JSON-LD between the markers.
    items = faq_items(index)
    block = ld_script({"@context": "https://schema.org", **faq_node(BASE + "/", items)}) if items else ""
    new_index = re.sub(
        r"(<!-- faq-jsonld:[^>]*-->\n).*?(<!-- /faq-jsonld -->)",
        lambda m: m.group(1) + (block + "\n" if block else "") + m.group(2),
        index, count=1, flags=re.S,
    )
    if "<!-- faq-jsonld:" not in index:
        sys.exit("build-pages: the <!-- faq-jsonld --> markers are missing in site/index.html")
    if write_if_changed(os.path.join(SITE, "index.html"), new_index):
        print("updated site/index.html (FAQ JSON-LD)")
    urls = [(BASE + "/", stamp(BASE + "/", new_index))]

    builds = [(slug, f"{slug}/index.html") for slug, _, _ in PAGES + [HUB] + ARTICLES] + OTHER
    for name, out_rel in builds:
        meta, body = parse_source(read(os.path.join(SRC, *name.split("/")) + ".html"))
        meta["_src"] = name + ".html"
        path = out_rel[: -len("index.html")] if out_rel.endswith("/index.html") else out_rel
        page, noindex = build_page(path, meta, body)
        out = os.path.join(SITE, *out_rel.split("/"))
        print(("wrote " if write_if_changed(out, page) else "unchanged ") + "site/" + out_rel)
        if not noindex:
            urls.append((f"{BASE}/{path}", stamp(f"{BASE}/{path}", page)))

    sitemap = '<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
    sitemap += "".join(f"  <url><loc>{u}</loc><lastmod>{d}</lastmod></url>\n" for u, d in urls)
    sitemap += "</urlset>\n"
    if write_if_changed(os.path.join(SITE, "sitemap.xml"), sitemap):
        print("updated site/sitemap.xml")
    write_if_changed(lastmod_path, json.dumps(lastmod, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
