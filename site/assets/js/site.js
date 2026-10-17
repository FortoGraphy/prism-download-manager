/* Prism website — vanilla JS, no dependencies.
   Theme · nav · OS detection + latest-release links · GitHub stars · engine demo ·
   reveal-on-scroll · tabs · copy buttons · film facade · hero video slot · accents. */
(function () {
  "use strict";

  var REPO = "FortoGraphy/prism-download-manager";
  var GH = "https://github.com/" + REPO;
  var API = "https://api.github.com/repos/" + REPO;
  var root = document.documentElement;
  var reduceMQ = window.matchMedia("(prefers-reduced-motion: reduce)");
  var $ = function (s, el) { return (el || document).querySelector(s); };
  var $$ = function (s, el) { return Array.prototype.slice.call((el || document).querySelectorAll(s)); };

  function store(kind) {
    try { var s = window[kind]; s.setItem("__t", "1"); s.removeItem("__t"); return s; } catch (e) { return null; }
  }
  var LS = store("localStorage");
  var SS = store("sessionStorage");

  /* ------------------------------------------------------------------ theme */
  var toggle = $(".theme-toggle");
  function applyThemeChrome(t) {
    if (toggle) toggle.setAttribute("aria-label", t === "dark" ? "Switch to light theme" : "Switch to dark theme");
    $$('meta[name="theme-color"]').forEach(function (m) { m.setAttribute("content", t === "dark" ? "#07080b" : "#f4f5f8"); });
  }
  function setTheme(t, save) {
    root.setAttribute("data-theme", t);
    if (save && LS) LS.setItem("prism-theme", t);
    applyThemeChrome(t);
    document.dispatchEvent(new CustomEvent("prism:theme"));
  }
  applyThemeChrome(root.getAttribute("data-theme"));
  if (toggle) toggle.addEventListener("click", function () {
    setTheme(root.getAttribute("data-theme") === "dark" ? "light" : "dark", true);
  });
  window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", function (e) {
    var saved = LS && LS.getItem("prism-theme");
    if (saved !== "light" && saved !== "dark") setTheme(e.matches ? "light" : "dark", false);
  });

  /* ---------------------------------------------------------------- accents */
  var swatches = $$("[data-accent-pick]");
  function setAccent(a) {
    root.setAttribute("data-accent", a);
    swatches.forEach(function (b) { b.setAttribute("aria-pressed", String(b.getAttribute("data-accent-pick") === a)); });
    if (SS) SS.setItem("prism-accent", a);
    document.dispatchEvent(new CustomEvent("prism:theme"));
  }
  swatches.forEach(function (b) { b.addEventListener("click", function () { setAccent(b.getAttribute("data-accent-pick")); }); });
  var savedAccent = SS && SS.getItem("prism-accent");
  if (savedAccent) setAccent(savedAccent);

  /* -------------------------------------------------------------------- nav */
  var nav = $(".nav");
  var menuBtn = $(".nav-menu-btn");
  function closeMenu() {
    if (!nav.classList.contains("is-open")) return;
    nav.classList.remove("is-open");
    menuBtn.setAttribute("aria-expanded", "false");
    menuBtn.setAttribute("aria-label", "Open menu");
    menuBtn.querySelector("use").setAttribute("href", "#i-menu");
  }
  if (menuBtn) {
    menuBtn.addEventListener("click", function () {
      var open = !nav.classList.contains("is-open");
      nav.classList.toggle("is-open", open);
      menuBtn.setAttribute("aria-expanded", String(open));
      menuBtn.setAttribute("aria-label", open ? "Close menu" : "Open menu");
      menuBtn.querySelector("use").setAttribute("href", open ? "#i-close" : "#i-menu");
    });
    $$(".nav-links a").forEach(function (a) { a.addEventListener("click", closeMenu); });
    document.addEventListener("keydown", function (e) { if (e.key === "Escape") closeMenu(); });
  }

  var showcase = $(".showcase-stage");
  var ticking = false;
  function onScroll() {
    if (ticking) return;
    ticking = true;
    requestAnimationFrame(function () {
      ticking = false;
      root.classList.toggle("is-scrolled", window.scrollY > 8);
      if (showcase && !reduceMQ.matches) {
        var r = showcase.getBoundingClientRect();
        var vh = window.innerHeight;
        var p = Math.min(1, Math.max(0, 1 - (r.top - vh * 0.18) / (vh * 0.62)));
        showcase.style.setProperty("--tilt", (14 * (1 - p)).toFixed(2) + "deg");
        showcase.style.setProperty("--tilt-scale", (0.96 + 0.04 * p).toFixed(4));
      }
    });
  }
  window.addEventListener("scroll", onScroll, { passive: true });
  window.addEventListener("resize", onScroll, { passive: true });
  onScroll();

  // Highlight the nav link of the section in view
  if ("IntersectionObserver" in window) {
    var links = {};
    $$(".nav-links a").forEach(function (a) { links[a.getAttribute("href").slice(1)] = a; });
    var secObs = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        var a = links[en.target.id];
        if (!a) return;
        if (en.isIntersecting) {
          Object.keys(links).forEach(function (k) { links[k].removeAttribute("aria-current"); });
          a.setAttribute("aria-current", "true");
        } else if (a.getAttribute("aria-current")) {
          a.removeAttribute("aria-current");
        }
      });
    }, { rootMargin: "-45% 0px -50% 0px" });
    Object.keys(links).forEach(function (id) { var s = document.getElementById(id); if (s) secObs.observe(s); });
  }

  /* --------------------------------------------------------- reveal on scroll */
  var revealEls = $$(".reveal, [data-inview]");
  if ("IntersectionObserver" in window && !reduceMQ.matches) {
    var rObs = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (en.isIntersecting) { en.target.classList.add("is-in"); rObs.unobserve(en.target); }
      });
    }, { rootMargin: "0px 0px -4% 0px", threshold: 0.01 });
    revealEls.forEach(function (el) { rObs.observe(el); });
  } else {
    revealEls.forEach(function (el) { el.classList.add("is-in"); });
  }

  /* --------------------------------------------------------------- platform */
  function detectPlatform() {
    var ua = navigator.userAgent || "";
    var uad = navigator.userAgentData;
    var p = (uad && uad.platform) || navigator.platform || "";
    var info = { os: "unknown", arch: "", distro: "" };
    if (/iPhone|iPad|iPod/.test(ua) || (/Mac/.test(p) && navigator.maxTouchPoints > 1)) info.os = "ios";
    else if (/Android/i.test(ua)) info.os = "android";
    else if (/Win/i.test(p) || /Windows/.test(ua)) info.os = "windows";
    else if (/Mac/i.test(p) || /Mac OS X/.test(ua)) info.os = "mac";
    else if (/CrOS/.test(ua)) info.os = "chromeos";
    else if (/Linux|X11/i.test(p + " " + ua)) info.os = "linux";
    if (info.os === "linux") {
      if (/Fedora|Red Hat|SUSE|CentOS/i.test(ua)) info.distro = "rpm";
      else if (/Ubuntu|Debian|Mint/i.test(ua)) info.distro = "deb";
    }
    if (info.os === "mac") {
      info.arch = "arm"; // most Macs sold since 2020; refined below where the browser can tell
      try {
        var gl = document.createElement("canvas").getContext("webgl");
        var ext = gl && gl.getExtension("WEBGL_debug_renderer_info");
        var r = ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : "";
        if (/Intel|AMD|Radeon/i.test(r)) info.arch = "x64";
      } catch (e) {}
    }
    var browser = "chrome";
    if (/Firefox\//.test(ua)) browser = "firefox";
    else if (/Edg\//.test(ua)) browser = "edge";
    else if (/OPR\/|Vivaldi/.test(ua) || navigator.brave) browser = "other";
    info.browser = browser;
    return info;
  }

  var plat = detectPlatform();
  var refineArch = (navigator.userAgentData && navigator.userAgentData.getHighEntropyValues && plat.os === "mac")
    ? navigator.userAgentData.getHighEntropyValues(["architecture"]).then(function (v) {
        if (v && v.architecture) plat.arch = v.architecture === "arm" ? "arm" : "x64";
      }).catch(function () {})
    : Promise.resolve();

  var PICK = {
    windows: { key: "win-setup", name: "Windows", what: "Windows 10/11 installer" },
    mac: { key: "mac-arm", name: "macOS", what: "Apple Silicon .dmg" },
    linux: { key: "appimage", name: "Linux", what: ".AppImage (any distro)" }
  };
  // Direct links used until (or if) the GitHub API answers — clicking always downloads.
  var FALLBACK_DL = "https://github.com/FortoGraphy/prism-download-manager/releases/download/v1.0.8/";
  var FALLBACK_FILES = {
    "win-setup": "Prism_1.0.8_x64-setup.exe",
    "mac-arm": "Prism_1.0.8_aarch64.dmg",
    "mac-x64": "Prism_1.0.8_x64.dmg",
    "deb": "Prism_1.0.8_amd64.deb",
    "rpm": "Prism-1.0.8-1.x86_64.rpm",
    "appimage": "Prism_1.0.8_amd64.AppImage"
  };

  var ASSETS = {
    "win-setup": /_x64-setup\.exe$/i,
    "win-msi": /\.msi$/i,
    "win-zip": /windows-x64-portable\.zip$/i,
    "mac-arm": /_aarch64\.dmg$/i,
    "mac-x64": /_x64\.dmg$/i,
    "deb": /_amd64\.deb$/i,
    "rpm": /\.x86_64\.rpm$/i,
    "appimage": /\.AppImage$/i,
    "ext-chromium": /^prism-chromium\.zip$/i,
    "ext-firefox": /^prism-firefox\.zip$/i,
    "sums": /^SHA256SUMS\.txt$/i
  };

  function fmtSize(b) {
    if (!b && b !== 0) return "";
    if (b < 1e6) return Math.max(1, Math.round(b / 1e3)) + " KB";
    if (b < 1e9) return (b / 1e6).toFixed(1) + " MB";
    return (b / 1e9).toFixed(2) + " GB";
  }

  function choice() {
    var os = plat.os;
    if (!PICK[os]) return null;
    var c = { key: PICK[os].key, name: PICK[os].name, what: PICK[os].what, os: os };
    if (os === "mac" && plat.arch === "x64") { c.key = "mac-x64"; c.what = "Intel .dmg"; }
    if (os === "linux" && plat.distro === "deb") { c.key = "deb"; c.what = ".deb for Debian / Ubuntu"; }
    if (os === "linux" && plat.distro === "rpm") { c.key = "rpm"; c.what = ".rpm for Fedora / openSUSE"; }
    return c;
  }

  function renderPrimary(release) {
    var c = choice();
    var meta = $("[data-dl-meta]");
    var version = release && release.tag_name ? release.tag_name.replace(/^v?/, "v") : "";
    $$("[data-dl-primary]").forEach(function (a) {
      var label = a.querySelector("[data-dl-label]");
      if (!c) {
        label.textContent = (plat.os === "ios" || plat.os === "android") ? "Get Prism for desktop" : "Download Prism";
        a.setAttribute("href", "#download");
        return;
      }
      label.textContent = "Download for " + c.name;
      var asset = release && findAsset(release, c.key);
      a.setAttribute("href", asset ? asset.browser_download_url : FALLBACK_DL + FALLBACK_FILES[c.key]);
    });
    if (!meta) return;
    meta.textContent = "";
    var parts = [];
    if (c) {
      var asset = release && findAsset(release, c.key);
      if (version) parts.push(version);
      parts.push(c.what);
      if (asset) parts.push(fmtSize(asset.size));
    } else if (plat.os === "ios" || plat.os === "android") {
      parts.push("Prism is a desktop app for Windows, macOS and Linux");
    } else {
      parts.push("Windows · macOS · Linux — 64-bit");
    }
    meta.appendChild(document.createTextNode(parts.join(" · ") + " · "));
    var other = document.createElement("a");
    other.href = "#download";
    other.textContent = c && c.os === "mac" ? (c.key === "mac-x64" ? "Apple Silicon?" : "Intel Mac?") : "Other platforms";
    meta.appendChild(other);
  }

  function findAsset(release, key) {
    var re = ASSETS[key];
    if (!release || !release.assets || !re) return null;
    for (var i = 0; i < release.assets.length; i++) if (re.test(release.assets[i].name)) return release.assets[i];
    return null;
  }

  function renderRelease(release) {
    $$("[data-asset]").forEach(function (a) {
      var asset = findAsset(release, a.getAttribute("data-asset"));
      if (!asset) return;
      a.setAttribute("href", asset.browser_download_url);
      var f = a.querySelector("[data-file]");
      if (f) f.textContent = asset.name + " · " + fmtSize(asset.size);
    });
    if (release.tag_name) $$("[data-version]").forEach(function (el) { el.textContent = "Prism " + release.tag_name.replace(/^v?/, "v"); });
  }

  function highlightPlatform() {
    var c = choice();
    if (c) {
      var card = $('.dl-card[data-os="' + c.os + '"]');
      if (card) card.classList.add("is-you");
      var tab = $('[data-tabs="os"] [data-key="' + c.os + '"]');
      if (tab) selectTab(tab, false);
    }
    var bt = $('[data-tabs="browser"] [data-key="' + plat.browser + '"]');
    if (bt) selectTab(bt, false);
  }

  function getJSON(url, key, ttlMin) {
    var now = Date.now();
    if (LS) {
      try {
        var c = JSON.parse(LS.getItem(key) || "null");
        if (c && now - c.t < ttlMin * 60000) return Promise.resolve(c.d);
      } catch (e) {}
    }
    return fetch(url, { headers: { Accept: "application/vnd.github+json" } }).then(function (r) {
      if (!r.ok) throw new Error(r.status);
      return r.json();
    }).then(function (d) {
      if (LS) try { LS.setItem(key, JSON.stringify({ t: now, d: d })); } catch (e) {}
      return d;
    });
  }

  refineArch.then(function () {
    renderPrimary(null);
    highlightPlatform();
    getJSON(API + "/releases/latest", "prism-release-v2", 5).then(function (rel) {
      if (!rel || !rel.assets) return;
      var slim = { tag_name: rel.tag_name, assets: rel.assets.map(function (a) { return { name: a.name, size: a.size, browser_download_url: a.browser_download_url }; }) };
      renderRelease(slim);
      renderPrimary(slim);
    }).catch(function () { /* keep the releases-page fallback links */ });
  });

  // Stars
  getJSON(API, "prism-repo", 30).then(function (repo) {
    var n = repo && repo.stargazers_count;
    if (typeof n !== "number") return;
    var txt = n >= 1000 ? (n / 1000).toFixed(n >= 10000 ? 0 : 1).replace(/\.0$/, "") + "k" : String(n);
    $$("[data-stars]").forEach(function (el) { el.textContent = txt; el.hidden = false; });
    $$("[data-gh-star]").forEach(function (a) { a.setAttribute("aria-label", "Star Prism on GitHub — " + n.toLocaleString("en") + " stars"); });
  }).catch(function () {});

  /* ------------------------------------------------------------------- tabs */
  function selectTab(tab, focus) {
    var list = tab.closest("[role=tablist]");
    $$("[role=tab]", list).forEach(function (t) {
      var on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      var panel = document.getElementById(t.getAttribute("aria-controls"));
      if (panel) {
        panel.hidden = !on;
        if (on) panel.classList.add("is-in");
      }
    });
    if (focus) tab.focus();
  }
  $$("[role=tablist]").forEach(function (list) {
    var tabs = $$("[role=tab]", list);
    tabs.forEach(function (t, i) {
      t.addEventListener("click", function () { selectTab(t, false); });
      t.addEventListener("keydown", function (e) {
        var j = -1;
        if (e.key === "ArrowRight") j = (i + 1) % tabs.length;
        else if (e.key === "ArrowLeft") j = (i - 1 + tabs.length) % tabs.length;
        else if (e.key === "Home") j = 0;
        else if (e.key === "End") j = tabs.length - 1;
        if (j >= 0) { e.preventDefault(); selectTab(tabs[j], true); }
      });
    });
  });

  /* ------------------------------------------------------------ copy buttons */
  var live = document.createElement("div");
  live.className = "sr-only";
  live.setAttribute("aria-live", "polite");
  document.body.appendChild(live);
  $$(".copy-btn").forEach(function (b) {
    b.addEventListener("click", function () {
      var pre = b.parentElement.querySelector("pre");
      var text = pre ? pre.textContent.trim() : "";
      var done = function () {
        b.classList.add("done");
        live.textContent = "Copied to clipboard";
        setTimeout(function () { b.classList.remove("done"); live.textContent = ""; }, 1600);
      };
      if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text).then(done, function () {});
      else {
        var ta = document.createElement("textarea");
        ta.value = text; ta.style.position = "fixed"; ta.style.opacity = "0";
        document.body.appendChild(ta); ta.select();
        try { document.execCommand("copy"); done(); } catch (e) {}
        ta.remove();
      }
    });
  });


  /* ------------------------------------------------- after-download next steps */
  // A small card (never a blocker) that tells people what to do once the file is on disk.
  var NEXT_KIND = { "win-setup": "win", "win-msi": "win", "win-zip": "winzip", "mac-arm": "mac", "mac-x64": "mac", "deb": "deb", "rpm": "rpm", "appimage": "appimage" };
  function nextSteps(kind, file) {
    var f = "<code>" + file + "</code>";
    var guide = "Prism starts and <b>walks you through connecting your browser</b> &mdash; about a minute.";
    switch (kind) {
      case "win": return ["Open " + f + " from your Downloads.", "If Windows says <i>&ldquo;Windows protected your PC&rdquo;</i>, click <b>More info &rarr; Run anyway</b> (the installer isn&rsquo;t signed with a paid certificate).", guide];
      case "winzip": return ["Unzip " + f + " anywhere (a USB stick works).", "Run <code>prism.exe</code>. If SmartScreen appears: <b>More info &rarr; Run anyway</b>.", guide];
      case "mac": return ["Open " + f + ", drag <b>Prism</b> into <b>Applications</b> and the <b>Browser Extension</b> folder into Documents.", "First start only: in Terminal run <code>xattr -dr com.apple.quarantine /Applications/Prism.app</code> (or System Settings &rarr; Privacy &amp; Security &rarr; <b>Open Anyway</b>).", guide];
      case "deb": return ["Install it: <code>sudo apt install ./" + file + "</code>", "Start <b>Prism</b> from your applications menu.", guide];
      case "rpm": return ["Install it: <code>sudo dnf install ./" + file + "</code>", "Start <b>Prism</b> from your applications menu.", guide];
      default: return ["Make it executable: <code>chmod +x " + file + "</code>", "Run it: <code>./" + file + "</code>", guide];
    }
  }
  function showNext(kind, href) {
    var file = decodeURIComponent((href.split("?")[0].split("/").pop() || "Prism"));
    var old = $(".next-card");
    if (old) old.remove();
    var card = document.createElement("aside");
    card.className = "next-card";
    card.setAttribute("role", "status");
    card.innerHTML =
      '<button class="next-close" type="button" aria-label="Close">&times;</button>' +
      '<p class="next-eyebrow">Your download has started</p>' +
      "<h3>Next steps</h3>" +
      "<ol>" + nextSteps(kind, file).map(function (t) { return "<li>" + t + "</li>"; }).join("") + "</ol>" +
      '<p class="next-foot"><a href="#guide">Full install guide</a> &middot; <a class="next-again" href="' + href + '">Download didn&rsquo;t start?</a></p>';
    document.body.appendChild(card);
    requestAnimationFrame(function () { card.classList.add("in"); });
    var close = function () { card.classList.remove("in"); setTimeout(function () { card.remove(); }, 300); };
    $(".next-close", card).addEventListener("click", close);
    $('a[href="#guide"]', card).addEventListener("click", close);
    document.addEventListener("keydown", function esc(e) { if (e.key === "Escape") { close(); document.removeEventListener("keydown", esc); } });
  }
  document.addEventListener("click", function (e) {
    var a = e.target.closest && e.target.closest("a[data-dl-primary], a[data-asset]");
    if (!a || a.classList.contains("next-again")) return;
    var href = a.getAttribute("href") || "";
    if (!/\/releases\/download\//.test(href)) return;
    var key = a.getAttribute("data-asset");
    if (!key) {
      var c = choice();
      key = c && c.key;
    }
    var kind = NEXT_KIND[key];
    if (kind) showNext(kind, href);
  });

  /* -------------------------------------------------------------------- film */
  var film = $("[data-film]");
  if (film) {
    var playBtn = $(".film-play", film);
    playBtn.addEventListener("click", function () {
      var yt = film.getAttribute("data-youtube");
      if (yt) {
        // Privacy-enhanced embed, created only on click: nothing loads from YouTube before that.
        var f = document.createElement("iframe");
        f.src = "https://www.youtube-nocookie.com/embed/" + yt + "?autoplay=1&rel=0&playsinline=1";
        f.title = "Prism — the film";
        f.allow = "autoplay; encrypted-media; picture-in-picture; fullscreen";
        f.allowFullscreen = true;
        f.referrerPolicy = "strict-origin-when-cross-origin";
        film.replaceChildren(f);
        f.focus();
        return;
      }
      var src = film.getAttribute("data-src");
      var img = $("img", film);
      var v = document.createElement("video");
      v.controls = true;
      v.autoplay = true;
      v.playsInline = true;
      v.preload = "auto";
      if (img) v.poster = img.currentSrc || img.src;
      v.setAttribute("aria-label", "Prism — promo film");
      var s = document.createElement("source");
      s.src = src;
      s.type = "video/mp4";
      v.appendChild(s);
      var fail = function () {
        if ($(".film-msg", film)) return;
        var msg = document.createElement("p");
        msg.className = "film-msg";
        msg.innerHTML = 'The film arrives with the v1.0.0 release. <a href="' + GH + '/releases">Open the releases page</a>';
        film.appendChild(msg);
      };
      s.addEventListener("error", fail);
      v.addEventListener("error", fail);
      film.replaceChildren(v);
      var pr = v.play();
      if (pr && pr.catch) pr.catch(function () {});
      v.focus();
    });
  }

  /* ------------------------------------------------------- hero video slot */
  var hv = $(".hero-video");
  var saveData = navigator.connection && navigator.connection.saveData;
  if (hv && root.getAttribute("data-hero-video") === "on" && !reduceMQ.matches && !saveData) {
    $$("source", hv).forEach(function (s) { s.src = s.getAttribute("data-src"); });
    hv.load();
    hv.addEventListener("playing", function () { hv.classList.add("is-playing"); }, { once: true });
    var p = hv.play();
    if (p && p.catch) p.catch(function () {});
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (en) {
        if (en[0].isIntersecting) { var q = hv.play(); if (q && q.catch) q.catch(function () {}); } else hv.pause();
      }).observe(hv);
    }
  }

  /* ------------------------------------------------------ key visual slot */
  var kv = $(".hero-kv");
  // The rendered 3D key visual is dark art: it is used in the dark theme only
  // (the light theme keeps the vector prism). Loaded on first need, so light-theme
  // visitors never download it.
  function loadKV() {
    if (!kv || kv.getAttribute("data-loaded") || root.getAttribute("data-theme") !== "dark") return;
    kv.setAttribute("data-loaded", "1");
    var kvImg = $("img", kv), kvSrc = $("source", kv);
    kvImg.addEventListener("load", function () { kv.hidden = false; }, { once: true });
    if (kvSrc) kvSrc.srcset = kvSrc.getAttribute("data-srcset");
    kvImg.srcset = kvImg.getAttribute("data-srcset");
    kvImg.src = kvImg.getAttribute("data-src");
  }
  if (kv && root.getAttribute("data-hero-art") === "kv") {
    loadKV();
    document.addEventListener("prism:theme", loadKV);
  }

  /* ----------------------------------------------------- benchmark tooltips */
  var tip = $("#tip");
  if (tip && window.matchMedia("(hover: hover)").matches) {
    $$(".bar-fill[data-tip]").forEach(function (bar) {
      bar.addEventListener("pointerenter", function () { tip.textContent = bar.getAttribute("data-tip"); tip.hidden = false; requestAnimationFrame(function () { tip.classList.add("on"); }); });
      bar.addEventListener("pointermove", function (e) {
        var x = Math.min(e.clientX + 14, window.innerWidth - tip.offsetWidth - 8);
        tip.style.left = x + "px";
        tip.style.top = (e.clientY - tip.offsetHeight - 12) + "px";
      });
      bar.addEventListener("pointerleave", function () { tip.classList.remove("on"); tip.hidden = true; });
    });
  }

  /* ------------------------------------------------------------ engine demo */
  var card = $("#engine-demo");
  if (card) engineDemo(card);

  function engineDemo(card) {
    var N = 16, SHOWN = 8, SIZE = 6.2e9, SPEEDUP = 11, TICK = 250, MIN_SPLIT = 24e6;
    var q = function (k) { return card.querySelector('[data-ec="' + k + '"]'); };
    var el = { pct: q("pct"), speed: q("speed"), eta: q("eta"), conns: q("conns"), bar: q("bar"), map: q("map"), segs: q("segs"), more: q("more") };
    var lanes = $$(".lane", q("lanes")).map(function (li) {
      return { fill: li.querySelector(".lane-fill"), range: li.querySelector(".lane-range"), speed: li.querySelector(".lane-speed") };
    });
    var ctx = el.map.getContext("2d");
    var colors = [], track = "";
    function readColors() {
      var cs = getComputedStyle(card);
      colors = [];
      for (var i = 1; i <= 8; i++) colors.push(cs.getPropertyValue("--spectrum-" + i).trim() || "#8b7bff");
      track = cs.getPropertyValue("--track-bg").trim() || "rgba(255,255,255,.08)";
    }
    document.addEventListener("prism:theme", function () { readColors(); draw(); });

    var segs, conns, finishedAt;
    function reset(initial) {
      segs = []; conns = [];
      var len = SIZE / N;
      for (var i = 0; i < N; i++) {
        var s = { start: i * len, end: (i + 1) * len, conn: i };
        s.pos = s.start + (initial ? len * (0.22 + Math.random() * 0.35) : 0);
        segs.push(s);
        conns.push({ seg: s, base: 1.05e6 + Math.random() * 0.85e6, speed: 0 });
      }
      finishedAt = 0;
      card.classList.remove("is-done");
    }

    function fmtB(b) {
      if (b < 1e6) return Math.round(b / 1e3) + " KB";
      if (b < 1e9) return Math.round(b / 1e6) + " MB";
      return (b / 1e9).toFixed(1) + " GB";
    }
    function unit(b) { return b < 1e6 ? "KB" : b < 1e9 ? "MB" : "GB"; }
    function fmtRange(a, b) {
      if (a < 1) return "0 B–" + fmtB(b);
      if (unit(a) === unit(b)) return fmtB(a).split(" ")[0] + "–" + fmtB(b);
      return fmtB(a) + "–" + fmtB(b);
    }
    function fmtSpeed(b) { return b < 1e6 ? Math.round(b / 1e3) + " KB/s" : (b / 1e6).toFixed(1) + " MB/s"; }
    function fmtEta(s) {
      if (!isFinite(s)) return "—";
      s = Math.round(s);
      if (s >= 3600) return Math.floor(s / 3600) + "h " + Math.floor((s % 3600) / 60) + "m";
      if (s >= 60) return Math.floor(s / 60) + "m " + String(s % 60).padStart(2, "0") + "s";
      return s + "s";
    }

    function step() {
      var total = 0, active = 0;
      conns.forEach(function (c, i) {
        var s = c.seg;
        if (!s) { c.speed = 0; return; }
        c.speed = c.base * (0.82 + Math.random() * 0.36);
        s.pos = Math.min(s.end, s.pos + c.speed * SPEEDUP * TICK / 1000);
        total += c.speed; active++;
        if (s.pos >= s.end) {
          // work stealing: take the second half of the largest remaining range
          var best = null, bestLeft = 0;
          conns.forEach(function (o) {
            if (o.seg && o !== c) { var left = o.seg.end - o.seg.pos; if (left > bestLeft) { bestLeft = left; best = o; } }
          });
          if (best && bestLeft > MIN_SPLIT) {
            var mid = best.seg.pos + bestLeft / 2;
            var ns = { start: mid, end: best.seg.end, pos: mid, conn: i };
            best.seg.end = mid;
            segs.push(ns);
            c.seg = ns;
          } else {
            c.seg = null;
          }
        }
      });
      return { total: total, active: active };
    }

    function draw() {
      var w = el.map.clientWidth, h = el.map.clientHeight, dpr = Math.min(2, window.devicePixelRatio || 1);
      if (!w) return;
      if (el.map.width !== Math.round(w * dpr)) { el.map.width = Math.round(w * dpr); el.map.height = Math.round(h * dpr); }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = track;
      roundRect(0, 0, w, h, h / 2); ctx.fill();
      ctx.save();
      roundRect(0, 0, w, h, h / 2); ctx.clip();
      segs.forEach(function (s) {
        var x0 = s.start / SIZE * w, x1 = s.pos / SIZE * w;
        if (x1 - x0 < 0.3) return;
        var activeSeg = conns[s.conn].seg === s;
        ctx.globalAlpha = activeSeg ? 1 : 0.55;
        ctx.fillStyle = colors[s.conn % 8];
        ctx.fillRect(x0, 0, Math.max(0.6, x1 - x0 - 0.8), h);
        if (activeSeg) { ctx.globalAlpha = 1; ctx.fillStyle = "#fff"; ctx.fillRect(Math.max(x0, x1 - 1.6), 0, 1.6, h); }
      });
      ctx.restore();
      ctx.globalAlpha = 1;
    }
    function roundRect(x, y, w, h, r) {
      ctx.beginPath();
      ctx.moveTo(x + r, y); ctx.arcTo(x + w, y, x + w, y + h, r); ctx.arcTo(x + w, y + h, x, y + h, r);
      ctx.arcTo(x, y + h, x, y, r); ctx.arcTo(x, y, x + w, y, r); ctx.closePath();
    }

    function render(st) {
      var done = 0;
      segs.forEach(function (s) { done += s.pos - s.start; });
      var frac = Math.min(1, done / SIZE);
      el.pct.textContent = (frac * 100).toFixed(1);
      el.bar.style.setProperty("--p", frac.toFixed(4));
      el.speed.textContent = st.active ? fmtSpeed(st.total) : "—";
      el.eta.textContent = st.active ? fmtEta((SIZE - done) / st.total) : "—";
      el.conns.textContent = st.active + " / " + N;
      el.segs.textContent = segs.length + " segments";
      lanes.forEach(function (l, i) {
        var c = conns[i], s = c.seg;
        if (!s) { l.fill.style.setProperty("--p", "1"); l.range.textContent = "done"; l.speed.textContent = "—"; return; }
        var p = (s.pos - s.start) / Math.max(1, s.end - s.start);
        l.fill.style.setProperty("--p", p.toFixed(4));
        l.range.textContent = fmtRange(s.start, s.end);
        l.speed.textContent = fmtSpeed(c.speed);
      });
      draw();
      return frac;
    }

    readColors();
    reset(true);
    // initial speeds for the first paint
    conns.forEach(function (c) { c.speed = c.base; });
    render({ total: conns.reduce(function (a, c) { return a + c.speed; }, 0), active: N });

    if (reduceMQ.matches) return; // static, information-only frame

    var timer = null, visible = false;
    function tick() {
      if (finishedAt) {
        if (Date.now() - finishedAt > 2600) { reset(false); }
        return;
      }
      var st = step();
      var frac = render(st);
      if (frac >= 1 || st.active === 0) {
        finishedAt = Date.now();
        card.classList.add("is-done");
        el.pct.textContent = "100";
        el.speed.textContent = "Done";
        el.eta.textContent = "—";
      }
    }
    function run() {
      var should = visible && document.visibilityState === "visible";
      if (should && !timer) timer = setInterval(tick, TICK);
      if (!should && timer) { clearInterval(timer); timer = null; }
    }
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (en) { visible = en[0].isIntersecting; run(); }, { rootMargin: "100px" }).observe(card);
    } else { visible = true; run(); }
    document.addEventListener("visibilitychange", run);
    window.addEventListener("resize", draw, { passive: true });
  }
})();
