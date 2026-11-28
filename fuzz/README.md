# Fuzz targets

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets for the parsers that read what a
server sends. They need a nightly toolchain:

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run video_index corpus/video_index seeds/video_index -- -dict=dict/video_index.dict
cargo +nightly fuzz run file_name corpus/file_name -- -dict=dict/file_name.dict
```

Run them from this directory. `corpus/` collects the inputs the fuzzer finds interesting,
`artifacts/` the ones that crash; both stay out of git.

| Target | What it feeds | What must hold |
|---|---|---|
| `video_index` | the start of a video file to `prism_video::index::parse` | no panic, and a part's span is never empty |
| `file_name` | a `Content-Disposition` header and a URL path to the file-name derivation | the name is never empty, at most 200 bytes, with no `/`, `\`, control character or trailing dot or space |

## Seeds and dictionaries

`seeds/video_index` holds the start of small files made with ffmpeg, cut right after their index:

```sh
ffmpeg -f lavfi -i testsrc=d=6:s=64x48:r=10 -c:v libx264 -g 10 \
  -movflags +frag_keyframe+empty_moov+default_base_moof+global_sidx fragmented.mp4
ffmpeg -f lavfi -i testsrc=d=6:s=64x48:r=10 -f lavfi -i sine=d=6 -c:v libx264 -g 10 -c:a aac \
  -shortest -f dash -single_file 1 -seg_duration 1 dash.mpd   # dash-video.mp4, dash-audio.mp4
ffmpeg -f lavfi -i testsrc=d=6:s=64x48:r=10 -c:v libvpx-vp9 -g 10 -deadline realtime \
  -dash 1 -cues_to_front 1 -reserve_index_space 2048 -f webm vp9.webm
ffmpeg -f lavfi -i sine=d=6 -c:a libopus -dash 1 -cues_to_front 1 -reserve_index_space 1024 \
  -f webm opus.webm
```

`dict/` lists the box names, EBML IDs and header fragments the fuzzer would take long to guess.

## A crash

The targets build with debug assertions, so an overflow panics even where a release build would
wrap around silently. To see what a crash does in a release build:

```sh
cargo +nightly fuzz build -O --target-dir target-release
target-release/x86_64-unknown-linux-gnu/release/<target> artifacts/<target>/crash-...
```

A crash in a release build of a parser that reads server data may be a vulnerability: report it
privately, as [SECURITY.md](../SECURITY.md) explains, not in a public issue or pull request.
