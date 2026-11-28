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
* the component: app, download engine, native messaging host, browser extension, updater, or
  the build and release workflows;
* steps or a proof of concept to reproduce it;
* the impact: who is affected, and under which conditions.

## What happens next

* You get a first answer as soon as possible, usually within two weeks.
* Once the issue is confirmed, we agree on a fix and keep you posted in the advisory.
* The advisory is published when the fixed version is out. Reporters are credited, unless they
  prefer not to be.

## Scope

In scope: the desktop app, the download, video and torrent engines, the native messaging host
and its browser registration, the browser extension, the local connection between them, the
in-app updater, and the build and release workflows.

Out of scope:

* bugs in yt-dlp, ffmpeg or deno themselves: please report them upstream, and tell us too if
  the way Prism uses them makes them exploitable;
* attacks that need an already compromised account or administrator access to the machine;
* scanner output without a demonstrated impact.

## Verify a download

From 1.2.8 on, every file of a release comes with a build provenance attestation: a record,
signed through Sigstore, of the repository, workflow and tag that built it. To check a file
with the [GitHub CLI](https://cli.github.com/), replacing the file name and the version:

```sh
gh attestation verify Prism_1.2.8_amd64.AppImage -R FortoGraphy/prism-download-manager \
  --signer-workflow FortoGraphy/prism-download-manager/.github/workflows/release.yml \
  --source-ref refs/tags/v1.2.8
```

It fails if the file changed after the build, or if anything other than the release workflow
of this repository built it from that tag. Without the last two options, it only checks that a
workflow of this repository built it.

`SHA256SUMS.txt`, in the same release, lists the hash of every file. Checking it catches a
damaged download, not a replaced one, since it comes from the same place as the files:

```sh
sha256sum -c --ignore-missing SHA256SUMS.txt
```

On Windows or macOS, compare the line of your file with `Get-FileHash <file>` (PowerShell) or
`shasum -a 256 <file>`. Updates installed from inside Prism don't need any of this: the updater
checks them against Prism's update signing key before installing them.
