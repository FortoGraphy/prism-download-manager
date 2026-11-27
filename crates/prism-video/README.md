# prism-video

Video backend for Prism: a self-contained abstraction over **yt-dlp + ffmpeg (+ deno)**.
It has no dependency on `prism-engine` or `prism-core`. The core uses it like this:

```rust
let svc = VideoService::new(app_data.join("tools"), VideoOptions::default());
svc.ensure_tools(|p| ui.tool_progress(p)).await?;           // first run: SHA-256-verified downloads
let Probe::Video(info) = svc.probe(url, &ctx).await? else { /* playlist picker */ };
let choice = &info.choices[0];                                 // the quality picker list
match svc.plan(&info, choice, &output_opts, &ctx) {
    Plan::Accelerated(mut p) => {
        // Download p.streams with the segmented engine (exact headers, ranges <= chunk_size),
        // then point the post-process inputs at the files:
        p.post.set_input_dir(&temp_dir);
        let file = svc.postprocess(&p.post, Some(progress_tx), cancel).await?;
    }
    Plan::YtDlp(job) => {
        let file = svc.run_ytdlp(&job, progress_tx, cancel).await?;
    }
}
// If the accelerated download fails (e.g. a 403 on a googlevideo URL):
let job = svc.plan_ytdlp_forced(&info, choice, &output_opts, &ctx);
```

Every type that reaches the UI is `serde` Serialize/Deserialize with `camelCase` fields.
Enums use camelCase variants. `Probe` and `Plan` are internally tagged (`kind` / `mode`).
`UpdateOutcome` is tagged by `status`.

## Public API

```rust
impl VideoService {                                     // Clone (Arc inside)
    pub fn new(tools_dir: PathBuf, opts: VideoOptions) -> Self;
    pub fn set_options(&self, opts: VideoOptions);
    pub fn options(&self) -> VideoOptions;
    pub fn tools_dir(&self) -> &Path;

    pub async fn tool_status(&self) -> ToolStatus;
    pub async fn ensure_tools(&self, progress: impl Fn(ToolProgress) + Send + Sync) -> Result<ToolStatus>;
    pub async fn install_tool(&self, tool: Tool, progress: impl Fn(ToolProgress) + Send + Sync) -> Result<ToolInfo>;
    pub async fn update_ytdlp(&self) -> Result<UpdateOutcome>;

    pub async fn probe(&self, url: &str, ctx: &RequestContext) -> Result<Probe>;
    pub fn choices(&self, info: &VideoInfo, container: Container) -> Vec<Choice>;
    pub fn plan(&self, info: &VideoInfo, choice: &Choice, opts: &OutputOptions, ctx: &RequestContext) -> Plan;
    pub fn plan_ytdlp_forced(&self, info: &VideoInfo, choice: &Choice, opts: &OutputOptions, ctx: &RequestContext) -> YtDlpJob;
    pub fn ytdlp_command_line(&self, job: &YtDlpJob, cookie_file: Option<&Path>) -> Vec<String>;

    pub async fn run_ytdlp(&self, job: &YtDlpJob, progress: UnboundedSender<YtProgress>, cancel: CancellationToken) -> Result<PathBuf>;
    pub async fn postprocess(&self, job: &PostProcess, progress: Option<UnboundedSender<YtProgress>>, cancel: CancellationToken) -> Result<PathBuf>;
}
```

These are the differences from the original sketch:

* `plan` also takes the `RequestContext`. Stream headers need the page's cookies and referer.
* `postprocess` takes an optional progress sender. ffmpeg's `-progress pipe:1` is reported as `YtProgress { stage: Merging | ExtractingAudio, percent }`.
* `CancellationToken` is re-exported from `tokio_util`.

The main types are:

| Type | Purpose |
|---|---|
| `VideoOptions` | `ytdlpPath` / `ffmpegPath` / `denoPath` overrides. Empty strings count as unset. Also `channel` (`stable` or `nightly`, default nightly), `cookiesFromBrowser`, `userAgent`, `proxy`, `preferSystemFfmpeg` (default true), `concurrentFragments` (8), `deleteTempInputs` (true) and `extraYtdlpArgs`. |
| `RequestContext` | Browser context for the page: `cookies` (a `Cookie` header string), `referer`, `userAgent`, extra `headers`, and `wantPlaylist`. |
| `ToolStatus` / `ToolInfo` | Per tool: `path`, `version`, `source` (`managed`, `system`, `custom` or `missing`) and a UI `message`. |
| `ToolProgress` | `{ tool, phase: resolving \| downloading \| verifying \| extracting \| installed, downloaded, total }` |
| `Probe` | `video(VideoInfo)` or `playlist(PlaylistInfo { title, entries: [{ id, url, title, duration, thumbnail, uploader }] })` |
| `VideoInfo` | `id`, `title`, `uploader`, `channel`, `duration`, `uploadDate`, `description`, `thumbnail`, `webpageUrl`, `extractor`, `isLive`, `liveStatus`, `chapters`, `subtitles` (manual and auto, `auto: true`), `formats`, `choices`, `hasDrm` |
| `Format` | yt-dlp format plus derived fields: `kind` (`video`, `audio`, `muxed` or `other`), `usable`, `fragmented`, `httpChunkSize` and `cookies` |
| `Choice` | One quality-picker entry: `id`, `kind`, `label` (e.g. "2160p60 · HDR · AV1"), `subLabel` (e.g. "MP4 · ~743 MB"), `height`, `fps`, `hdr`, codecs, `estSize`, `videoFormatId` / `audioFormatId` / `formatId`, `container`, `audioFormat`, `accelerable` |
| `OutputOptions` | `container`, `audioFormat`, `mp3BitrateKbps`, subtitle embed/write options plus `subLangs` and `autoSubtitles`, thumbnail embed/write, `embedMetadata`, `embedChapters`, `sponsorblock { mode: off \| mark \| remove, categories }`, `fileTemplate` and `outputDir` |
| `Plan` | `accelerated { streams: [StreamSpec], post: PostProcess }` or `ytDlp(YtDlpJob { url, args, outputDir, context, reason })` |
| `StreamSpec` | `role` (`video`, `audio`, `muxed`, `subtitle` or `thumbnail`), `url`, `headers`, `size` / `sizeExact`, `ext`, `fileName`, `formatId`, **`chunkSize`**, `language`, `optional` |
| `PostProcess` | `inputs[]` (role, fileName, and a **`path` the core fills in**), `outputPath`, `container`, `audioConversion`, `metadata`, `chapters`, embed/write flags, `sponsorblock`, `duration`, `deleteInputs` |
| `YtProgress` | `stage` (one of `starting`, `downloading`, `merging`, `extractingAudio`, `converting`, `embeddingSubtitles`, `embeddingThumbnail`, `embeddingMetadata`, `sponsorBlock`, `moving`, `postProcessing` or `finished`), plus `downloaded`, `total`, `totalIsEstimate`, `speed`, `eta`, `fragmentIndex` / `fragmentCount`, `filename`, `percent`, `postprocessor` and `status` |

## Tools (`tools.rs`)

Each tool is resolved in this order: **custom path → managed copy in `tools_dir` → `PATH` → missing.**
A custom ffmpeg path may be a directory. ffprobe is looked up next to it.

| Tool | Source | Verification |
|---|---|---|
| yt-dlp | GitHub `yt-dlp/yt-dlp-nightly-builds` (nightly) or `yt-dlp/yt-dlp` (stable). Assets: `yt-dlp.exe`, `yt-dlp_arm64.exe`, `yt-dlp_x86.exe`, `yt-dlp_linux`, `yt-dlp_linux_aarch64`, `yt-dlp_macos` | `SHA2-256SUMS` from the **same tag**. `releases/latest` is first resolved to a concrete tag with a non-following request, so the checksum file and the binary cannot come from different releases. |
| deno | `denoland/deno` latest `deno-<triple>.zip` | `<asset>.sha256sum`. The Windows files are PowerShell `Get-FileHash` output, not GNU format, and the parser handles both. |
| ffmpeg + ffprobe (Windows, Linux) | BtbN `FFmpeg-Builds` `latest`: `ffmpeg-master-latest-{win64,winarm64}-gpl.zip` and `ffmpeg-master-latest-{linux64,linuxarm64}-gpl.tar.xz` | `checksums.sha256`. The `.tar.xz` files are decoded in pure Rust with `lzma-rust2` and `tar`. |
| ffmpeg + ffprobe (macOS) | `ffmpeg.martin-riedl.de/redirect/latest/macos/{arm64,amd64}/release/{ffmpeg,ffprobe}.zip`. The redirect is resolved to the concrete build URL. | `<build url>.sha256`. `brew install ffmpeg` works too: a ffmpeg on PATH is preferred. |

Installs work like this:

* Downloads stream into a temp file in `tools_dir`, are hashed on the fly, verified, extracted and atomically renamed into place.
* Files get `chmod 755` on Unix. On macOS the quarantine attribute is removed (best effort).
* On Windows a running `yt-dlp.exe` is moved aside to `.old-N`, and the leftover is cleaned up on the next `VideoService::new`.
* `update_ytdlp` compares the managed file's SHA-256 with the channel's latest `SHA2-256SUMS` entry and re-downloads if they differ. This also switches channels. It never touches a system or custom yt-dlp: it returns `notManaged`.
* `ensure_tools` keeps a system ffmpeg when one is present, unless `preferSystemFfmpeg` is false.

Child processes:

* Every child process (yt-dlp, ffmpeg, deno, version checks) runs with `CREATE_NO_WINDOW` on Windows, `kill_on_drop`, `PYTHONUTF8=1` and `PYTHONIOENCODING=utf-8`.
* **Whole-tree kill.** The PyInstaller yt-dlp executable re-spawns itself, and yt-dlp spawns ffmpeg.
  * On Windows each child is put in a Job Object with `KILL_ON_JOB_CLOSE`. Cancel terminates the job.
  * On Unix each child gets its own process group. Cancel and drop send `SIGKILL` to the group.

## Probing (`probe.rs`, `model.rs`, `choices.rs`)

The probe command is:

```
yt-dlp --ignore-config [--ffmpeg-location D] [--js-runtimes deno:PATH] [--proxy P] [--cookies-from-browser B] [--cookies F] [--add-headers ...] -J --no-warnings --flat-playlist (--no-playlist|--yes-playlist) -- URL
```

* `--flat-playlist` is always passed. It does not affect single videos, but it keeps channel and playlist listings fast. For example, `@BlenderOfficial/videos` returns 1,215 entries in about 20 s.
* A `_type: url` redirect result is followed once.
* Browser cookies are written to a temporary Netscape `cookies.txt` for the page's site (`.example.com`). The file is deleted after the run.
* Referer and User-Agent go through `--add-headers`. The old `--referer` / `--user-agent` flags are deprecated and hidden from `--help`.
* The browser User-Agent is **not** forced on YouTube. yt-dlp uses per-client UAs there, and the stream URLs are bound to them.
* `--js-runtimes deno:<path>` was confirmed against `yt-dlp 2026.10.10.233141 --help` and with `-v` (`[debug] JS runtimes: deno-2.9.7`). The path must be a native Windows path.
* Parsing is tolerant. Numbers can arrive as strings, `has_drm` can be `"maybe"` (treated as not DRM), entries can be null or of the wrong type, and missing fields are fine.

### Choice selection

* **Video:** one entry per (height, HDR) pair, up to 4320p, highest first, with HDR before SDR. Within a bucket the order is:
  1. Highest fps.
  2. Codec preference for the container:
     * MP4: H.264 > AV1 > VP9 > HEVC.
     * MP4 HDR: AV1 > VP9 > HEVC.
     * MKV: VP9 > AV1 > H.264.
     * WebM: VP9 > AV1 > VP8.
  3. Separate video+audio before a pre-muxed file.
  4. Progressive HTTP before HLS/DASH.
  5. Bitrate.
* **Audio pairing:** the order is:
  1. Original language.
  2. Non-DRC.
  3. Codec for the container (MP4 AAC > Opus; WebM/MKV Opus first).
  4. HTTP.
  5. Bitrate.
* **Container fallback:** codecs the container cannot hold (H.264 in WebM, for example) make the choice fall back to MKV.
* **Audio-only:** MP3 320 kbps (converted), M4A (original AAC, or AAC 256 kbps), Opus (original, or 160 kbps), and FLAC.
* **DRM and other unusable formats:** DRM formats, storyboards and images are marked `usable: false`. A video where every format is DRM has `hasDrm: true` and no choices.

## Planning (`plan.rs`)

`Plan::Accelerated` is chosen when all of these hold:

* Every referenced format is a single `http`/`https` file. HLS, DASH and fragmented formats don't qualify.
* The video is not live, upcoming or post-live.
* Nothing is DRM-protected.
* SponsorBlock is not set to **Remove**. Mark is supported: Prism fetches the segments itself and adds them as chapters.
* No part of the video is asked for (`OutputOptions.section`). Fetching the whole file to keep a minute of it would waste the rest.

Otherwise the result is `Plan::YtDlp`, with `reason` set.

**Engine contract for accelerated streams:**

* Send exactly `headers`. They are yt-dlp's `http_headers`, plus cookies from the format's `cookies` field, plus the page cookies when the stream is on the same site, plus the Referer.
* Keep every range request ≤ `chunk_size`. YouTube uses 10 MiB and throttles or 403s larger ranges.
* The URLs are short-lived and IP-bound. Re-probe on expiry, or switch to `plan_ytdlp_forced` on 403.
* Subtitle and thumbnail streams are `optional`. A failure there must not fail the download, and `postprocess` skips missing optional inputs.

The yt-dlp job arguments are:

```
-f <id>+<id>/bv*[height<=H]+ba/b[height<=H]/b  (--merge-output-format C --remux-video C | -x --audio-format F --audio-quality Q)
-N 8 [--download-sections *S-E --downloader-args "ffmpeg:-progress pipe:1 -nostats"]
[--sub-langs L --write-subs --write-auto-subs --embed-subs] [--embed-thumbnail --convert-thumbnails jpg] [--write-thumbnail]
[--embed-metadata] [--embed-chapters] [--sponsorblock-mark|--sponsorblock-remove CATS]
--newline --progress --color never --progress-delta 0.5
--progress-template "download:PRISM %(progress)j" --progress-template "postprocess:PRISMPP %(progress)j"
--print "after_move:PRISMFILE %(filepath)s" --no-mtime --windows-filenames --no-playlist -P <dir> -o <template>
```

At run time the service prepends `--ignore-config`, the tool locations, the proxy, the cookies and the headers, and appends `extraYtdlpArgs -- URL`.

Note that `PRISMPP` lines arrive on **stderr** and `PRISM`/`PRISMFILE` lines arrive on stdout. Both streams are parsed.

**A part of a video** (`OutputOptions.section`, seconds; `Section::part_of` cuts it to the video and drops a range that is the whole video or shorter than a second): yt-dlp hands the part to ffmpeg, which fetches only that stretch. The part is copied, not encoded again (#135: encoding it kept every core busy): a video part starts on the keyframe at or before its start (up to a couple of seconds early); its end and audio are exact. (The accelerated path still cuts on the exact frame.) yt-dlp reports nothing while ffmpeg works, so ffmpeg writes `-progress` blocks to stdout and `PartProgress` turns them into the share of the part's time that is done (`percent`, with no `total`, since the size is unknown until the end). yt-dlp doesn't shift subtitles, chapters or SponsorBlock segments to a part, so a part gets none of them. The file name gets ` (1m23s–2m45s)` (`Section::file_suffix`).

`OutputOptions.fileTemplate` takes a subset of yt-dlp's template syntax (`template.rs`): the fields `title`, `id`, `ext`, `uploader`, `uploader_id`, `channel`, `artist`, `album`, `track`, `creator`, `upload_date`, `playlist_title`, `playlist_index`, `extractor`, `extractor_key` and `duration`; the fallback `%(artist,uploader)s`, `%(field|default)s`, `%(field&replacement)s`, a date format `%(upload_date>%Y-%m-%d)s`, a cut `%(title).50s`, `%(playlist_index)03d` and `%%`. In `render_template` unknown fields become `NA`, and `/` creates subdirectories. The app names a video with `render_name` (#145): a field without a value is left out together with the separator and the brackets around it, so `%(artist)s - %(title)s` without an artist gives just the title; fields only the page has fill in once the job has read it. File names use the same replacement characters as yt-dlp's `--windows-filenames` (`：`, `？`, `⧸`, and so on), so both paths produce identical names.

## Post-processing (`ffmpeg.rs`)

**Video output:**

* `-map 0:v:0 -map 1:a:0 -c copy`.
* Subtitles: `mov_text` for MP4, copied (WebVTT) for MKV, `webvtt` for WebM. Each gets an ISO 639-2 language tag and a title.
* Thumbnail: WebP is converted to JPEG first. MP4 gets an attached-pic stream (`-disposition:v:1 attached_pic`). MKV gets `-attach cover.jpg` with a mimetype. WebM gets no thumbnail.
* Metadata and chapters come from a generated FFMETADATA1 file, mapped with `-map_metadata` and `-map_chapters`. Metadata covers title, artist (uploader), date, comment (URL) and description/synopsis (trimmed to 2,000 characters).
* MP4 also gets `-movflags +faststart`.

**Audio output:**

| Format | Encoding | Cover art |
|---|---|---|
| MP3 | `libmp3lame -b:a 320k` (or `-q:a 0`) plus ID3v2.3 | Yes |
| M4A | Stream copy when the source is AAC, otherwise `aac 256k` | Yes |
| Opus | Stream copy when the source is Opus, otherwise `libopus` | No |
| FLAC | `flac` | Yes |
| WAV | `pcm_s16le` | No |

**SponsorBlock Mark (accelerated path):**

* Uses the privacy-preserving hash-prefix endpoint, `/api/skipSegments/<sha256(id)[:4]>`.
* Existing chapters are split around the segments.
* Segments become `[SponsorBlock]: <Category>` chapters, and gaps become untitled chapters.

**Running:**

* Progress is read from `-progress pipe:1` (`out_time_us` against the duration).
* Output goes to `<name>.prismtmp.<ext>` and is renamed into place on success.
* Cancel kills ffmpeg and removes the partial output.
* Inputs are deleted on success when `deleteInputs` is set.
* `write_subtitles` and `write_thumbnail` keep `<stem>.<lang>.vtt` and `<stem>.jpg`.

## Tests

```
cargo test -p prism-video                                   # 41 offline tests (6 run real local ffmpeg; skipped if ffmpeg is missing)
cargo test -p prism-video --test live -- --ignored --nocapture --test-threads 1   # 7 network tests
cargo test -p prism-video --lib -- --ignored --nocapture                          # 2 network tests (BtbN Linux .tar.xz, macOS ffprobe)
```

The fixtures in `tests/fixtures/` are:

* `youtube_bbb.json`: real `yt-dlp -J` output for Big Buck Bunny (`aqz-KE-bpKQ`, CC-BY), captured with deno enabled and trimmed to 24 formats. Stream URLs are redacted.
* `youtube_flat_playlist.json`: real `--flat-playlist` output for `@BlenderOfficial/videos`, 3 entries.
* `generic_direct.json`: real generic-extractor output for a direct `.mp4`.
* `synthetic_hdr.json`: handcrafted. Covers HDR10 VP9 and AV1, 8K, DRM, `"maybe"` DRM, HLS-only, DRC audio, manual and auto subtitles, and chapters.

## Known limitations

* **Unix builds:** the Unix-only code paths (process groups, `chmod`, the macOS `xattr` call) were only checked by reading them. They have not been compiled. Cross-checking failed on this Windows host because `aws-lc-sys` needs a C cross-compiler. The macOS ffmpeg source and the Linux `.tar.xz` extraction *were* exercised from Windows, through the ignored tests.
* **Accelerated YouTube downloads** were verified with sequential 10 MiB ranges on one connection. Many parallel connections per googlevideo URL can be throttled. The engine should cap connections and keep the `plan_ytdlp_forced` fallback.
* **Cookies:** the Netscape file is scoped to the page host minus `www.`. Cookies that the browser holds for other domains (SSO, CDNs) are not passed. `cookiesFromBrowser` covers that, but Chrome's app-bound encryption on Windows can break it.
* **Cover art:** Opus and WAV outputs get no embedded cover in the accelerated path (yt-dlp uses mutagen for Opus).
* **Signatures:** checksum files come from the same host as the binaries. The yt-dlp `SHA2-256SUMS.sig` GPG signature is not verified.
* **BtbN rolling tag:** BtbN `latest` is a rolling tag. A checksum mismatch during a rebuild window shows up as `ChecksumMismatch`; retry later.
* **Subtitles:** only WebVTT, SRT and ASS are downloaded on the accelerated path. YouTube `json3` / `srv*` / `ttml` are skipped, since YouTube always offers `vtt` too.
