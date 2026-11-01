//! Linux (#41, #45, #49): Prism's sounds play through the system's own player (PipeWire,
//! PulseAudio or ALSA) instead of in the windows. WebKitGTK plays audio with GStreamer inside a
//! window's web process, and on some systems the windows stopped updating and took no clicks
//! right when a download finished or failed or a queue ended: the moments Prism plays a sound,
//! from every open window at once. Here a sound plays once, in a process of its own, and a
//! player that hangs can't take a window with it.
//!
//! Windows and macOS keep playing them in the window (`playSound` in app/src/lib/store.ts).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The sounds, as in app/public/sounds (the windows' copies are inside the app, not on disk).
const SOUNDS: &[(&str, &[u8])] = &[
    (
        "complete",
        include_bytes!("../../public/sounds/complete.wav"),
    ),
    ("failed", include_bytes!("../../public/sounds/failed.wav")),
    ("queue", include_bytes!("../../public/sounds/queue.wav")),
    (
        "complete-cascade",
        include_bytes!("../../public/sounds/complete-cascade.wav"),
    ),
    (
        "complete-droplet",
        include_bytes!("../../public/sounds/complete-droplet.wav"),
    ),
    (
        "complete-glide",
        include_bytes!("../../public/sounds/complete-glide.wav"),
    ),
    (
        "complete-harp",
        include_bytes!("../../public/sounds/complete-harp.wav"),
    ),
    (
        "complete-kalimba",
        include_bytes!("../../public/sounds/complete-kalimba.wav"),
    ),
    (
        "complete-music-box",
        include_bytes!("../../public/sounds/complete-music-box.wav"),
    ),
    (
        "complete-pop",
        include_bytes!("../../public/sounds/complete-pop.wav"),
    ),
    (
        "complete-prism",
        include_bytes!("../../public/sounds/complete-prism.wav"),
    ),
    (
        "complete-snap",
        include_bytes!("../../public/sounds/complete-snap.wav"),
    ),
    (
        "complete-sparkle",
        include_bytes!("../../public/sounds/complete-sparkle.wav"),
    ),
];

/// A player that hasn't finished a sound this long after starting it is stopped.
const PLAY_TIMEOUT: Duration = Duration::from_secs(10);

/// The sound (file name without .wav) that plays for `kind`, as `soundFile` in
/// app/src/lib/sounds.ts: an unknown complete sound falls back to the classic one.
pub fn sound_file(kind: &str, complete_sound: &str) -> &'static str {
    let name = match kind {
        "failed" => "failed".to_string(),
        "queue" => "queue".to_string(),
        _ => format!("complete-{complete_sound}"),
    };
    SOUNDS
        .iter()
        .map(|(n, _)| *n)
        .find(|n| *n == name)
        .unwrap_or("complete")
}

/// The players to try, in order, for `path` at `volume` (0–1).
fn players(path: &Path, volume: f32) -> Vec<(&'static str, Vec<String>)> {
    let file = path.to_string_lossy().into_owned();
    vec![
        (
            "pw-play",
            vec!["--volume".into(), format!("{volume:.2}"), file.clone()],
        ),
        (
            "paplay",
            vec![
                format!("--volume={}", (volume * 65536.0).round() as u32),
                file.clone(),
            ],
        ),
        ("aplay", vec!["-q".into(), file]),
    ]
}

/// Writes the sound next to Prism's data the first time it's needed.
fn sound_path(name: &str) -> Option<PathBuf> {
    let bytes = SOUNDS.iter().find(|(n, _)| *n == name)?.1;
    let dir = prism_core::paths::data_dir().join("sounds");
    let path = dir.join(format!("{name}.wav"));
    let current = std::fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64);
    if !current {
        std::fs::create_dir_all(&dir).ok()?;
        let tmp = dir.join(format!("{name}.wav.tmp"));
        std::fs::write(&tmp, bytes).ok()?;
        std::fs::rename(&tmp, &path).ok()?;
    }
    Some(path)
}

/// Plays `kind`'s sound on a thread of its own: the first player that is installed and plays it.
pub fn play(kind: &str, complete_sound: &str, volume: f32) {
    let name = sound_file(kind, complete_sound);
    let volume = if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        1.0
    };
    std::thread::spawn(move || {
        let Some(path) = sound_path(name) else {
            tracing::warn!("could not write the sound {name}");
            return;
        };
        for (exe, args) in players(&path, volume) {
            let child = Command::new(exe)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            let Ok(mut child) = child else {
                continue; // not installed
            };
            let started = Instant::now();
            let done = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status.success(),
                    Ok(None) if started.elapsed() < PLAY_TIMEOUT => {
                        std::thread::sleep(Duration::from_millis(50))
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break false;
                    }
                }
            };
            if done {
                return;
            }
            // Installed but no sound server for it (pw-play without PipeWire…): the next one.
        }
        tracing::info!("no audio player could play the {name} sound");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_sound_as_the_windows_would_play() {
        assert_eq!(sound_file("complete", "classic"), "complete");
        assert_eq!(sound_file("complete", "kalimba"), "complete-kalimba");
        assert_eq!(sound_file("complete", "music-box"), "complete-music-box");
        assert_eq!(sound_file("complete", "nope"), "complete");
        assert_eq!(sound_file("failed", "kalimba"), "failed");
        assert_eq!(sound_file("queue", "pop"), "queue");
        for (name, bytes) in SOUNDS {
            assert!(bytes.starts_with(b"RIFF"), "{name} is a WAV file");
        }
    }

    #[test]
    fn players_get_the_volume_they_understand() {
        let p = players(Path::new("/tmp/a.wav"), 0.6);
        assert_eq!(p[0].1, ["--volume", "0.60", "/tmp/a.wav"]);
        assert_eq!(p[1].1, ["--volume=39322", "/tmp/a.wav"]);
        assert_eq!(p[2].1, ["-q", "/tmp/a.wav"]);
    }
}
