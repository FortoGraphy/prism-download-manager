import { describe, expect, it } from "vitest";
import { COMPLETE_SOUNDS, soundFile } from "./sounds";

describe("download complete sounds", () => {
  it("plays the chosen complete sound and the fixed ones for the other kinds", () => {
    expect(soundFile("complete", "kalimba")).toBe("complete-kalimba");
    expect(soundFile("complete", "music-box")).toBe("complete-music-box");
    expect(soundFile("complete", "classic")).toBe("complete");
    expect(soundFile("failed", "kalimba")).toBe("complete-kalimba");
    expect(soundFile("queue", "kalimba")).toBe("complete-kalimba");
  });

  it("falls back to the classic sound for a missing or unknown choice", () => {
    expect(soundFile("complete")).toBe("complete");
    expect(soundFile("complete", "")).toBe("complete");
    expect(soundFile("complete", "../../etc")).toBe("complete");
  });

  it("ships a file for every choice", () => {
    const ids = COMPLETE_SOUNDS.map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
    const shipped = Object.keys(import.meta.glob("../../public/sounds/*.wav")).map((p) => p.split("/").pop());
    for (const { id } of COMPLETE_SOUNDS) expect(shipped).toContain(`${soundFile("complete", id)}.wav`);
  });
});
