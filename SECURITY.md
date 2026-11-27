# Security policy

## Supported versions

Only the latest release gets security fixes. Please check that the issue still happens on it
before reporting.

## Reporting a vulnerability

Please report it privately on GitHub:
[**Report a vulnerability**](https://github.com/FortoGraphy/prism-download-manager/security/advisories/new).
Don't open a public issue, discussion or pull request for it.

A useful report includes:

* the Prism version, the operating system, and the browser if the extension is involved;
* the component: app, prism-web, download engine, native messaging host, browser extension,
  updater, or the build and release workflows;
* steps or a proof of concept to reproduce it;
* the impact: who is affected, and under which conditions.

## What happens next

* You get a first answer as soon as possible, usually within two weeks.
* Once the issue is confirmed, we agree on a fix and keep you posted in the advisory.
* The advisory is published when the fixed version is out. Reporters are credited, unless they
  prefer not to be.

## Scope

In scope: the desktop app, prism-web, the download, video and torrent engines, the native
messaging host and its browser registration, the browser extension, the local connection between
them, the in-app updater, and the build and release workflows. Protect prism-web's bearer token
like a password, and use TLS (a reverse proxy) beyond loopback: plain HTTP exposes the token.

Out of scope:

* bugs in yt-dlp, ffmpeg or deno themselves: please report them upstream, and tell us too if
  the way Prism uses them makes them exploitable;
* attacks that need an already compromised account or administrator access to the machine;
* scanner output without a demonstrated impact.
