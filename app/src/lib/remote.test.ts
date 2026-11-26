import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({ core: false, event: false }));
vi.mock("@tauri-apps/api/core", () => {
  state.core = true;
  return { invoke: async () => { throw new Error("Tauri core loaded in web mode"); } };
});
vi.mock("@tauri-apps/api/event", () => {
  state.event = true;
  return { listen: async () => { throw new Error("Tauri events loaded in web mode"); } };
});

const jsonResponse = (body: unknown, status = 200): Response =>
  ({ ok: status >= 200 && status < 300, status, json: async () => body }) as Response;

let fetchMock: ReturnType<typeof vi.fn>;
let assign: ReturnType<typeof vi.fn>;
let store: Map<string, string>;

beforeEach(() => {
  vi.resetModules();
  state.core = false;
  state.event = false;
  fetchMock = vi.fn();
  assign = vi.fn();
  store = new Map();
  vi.stubGlobal("window", { __PRISM_WEB__: { auth: true } });
  vi.stubGlobal("location", { pathname: "/queue", hash: "#top", assign });
  vi.stubGlobal("sessionStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
  });
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("prism-web bridge", () => {
  it("posts cmd/args with the bearer token and returns the result", async () => {
    store.set("prism-web-token", "secret");
    fetchMock.mockResolvedValue(jsonResponse({ saved: true }));
    const { api } = await import("./api");

    expect(await api.updateSettings({ general: { theme: "dark" } })).toEqual({ saved: true });
    expect(fetchMock).toHaveBeenCalledWith("/api/invoke", {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: "Bearer secret" },
      body: JSON.stringify({ cmd: "update_settings", args: { patch: { general: { theme: "dark" } } } }),
    });
  });

  it("rejects with the server's error message", async () => {
    store.set("prism-web-token", "secret");
    fetchMock.mockResolvedValue(jsonResponse({ error: "Capture is gone" }, 409));
    const { api } = await import("./api");

    await expect(api.getCapture("x")).rejects.toThrow("Capture is gone");
  });

  it("redirects to login when auth is on and there is no token", async () => {
    const { api } = await import("./api");

    await expect(api.bootstrap()).rejects.toThrow("Authentication required");
    expect(assign).toHaveBeenCalledWith("/login.html?next=%2Fqueue%23top");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("streams core events and never loads the Tauri bridge", async () => {
    store.set("prism-web-token", "secret");
    let reads = 0;
    const reader = {
      read: async () => {
        reads += 1;
        if (reads === 1) return { done: false, value: new TextEncoder().encode('data: {"type":"showMain"}\n\n') };
        return { done: true, value: undefined };
      },
    };
    fetchMock.mockResolvedValue({ ok: true, status: 200, body: { getReader: () => reader } } as unknown as Response);
    const { onCoreEvent, onAppEvent } = await import("./api");

    const handler = vi.fn();
    const off = await onCoreEvent(handler);
    await vi.waitFor(() => expect(handler).toHaveBeenCalledWith({ type: "showMain" }));
    expect(fetchMock).toHaveBeenCalledWith("/api/events", {
      headers: { Authorization: "Bearer secret" },
      signal: expect.any(AbortSignal),
    });
    const offApp = await onAppEvent("update-progress", () => {});
    offApp();
    off();

    expect(state.core).toBe(false);
    expect(state.event).toBe(false);
  });

  it("reads /api/config when the page did not inject one", async () => {
    vi.stubGlobal("window", {});
    fetchMock.mockResolvedValue(jsonResponse({ auth: false }));
    const { config } = await import("./remote");

    expect(await config()).toEqual({ auth: false });
    expect(fetchMock).toHaveBeenCalledWith("/api/config");
  });
});
