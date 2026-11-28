import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { showAfterPaint } from "./show-window";

describe("window visibility", () => {
  let frames: Map<number, FrameRequestCallback>;
  let next: number;
  const paint = () => {
    const pending = [...frames.values()];
    frames.clear();
    pending.forEach((callback) => callback(0));
  };

  beforeEach(() => {
    vi.useFakeTimers();
    frames = new Map();
    next = 0;
    vi.stubGlobal("window", globalThis);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frames.set(++next, callback);
      return next;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("reveals only after the content has had a paint opportunity", () => {
    const show = vi.fn();
    showAfterPaint(show);
    paint();
    expect(show).not.toHaveBeenCalled();
    paint();
    expect(show).toHaveBeenCalledOnce();
    vi.advanceTimersByTime(1500);
    expect(show).toHaveBeenCalledOnce();
  });

  it("reveals a hidden WebKit window even when animation frames are suspended", () => {
    const show = vi.fn();
    showAfterPaint(show);
    vi.advanceTimersByTime(120);
    expect(show).toHaveBeenCalledOnce();
    paint();
    paint();
    expect(show).toHaveBeenCalledOnce();
  });

  it("does not reveal an unmounted window with a second frame pending", () => {
    const show = vi.fn();
    const cancel = showAfterPaint(show);
    paint();
    cancel();
    paint();
    vi.advanceTimersByTime(1500);
    expect(show).not.toHaveBeenCalled();
  });
});
