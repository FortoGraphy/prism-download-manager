// HTTP/SSE bridge used when prism-web serves the UI. No Tauri or app imports here.

import type { CoreEvent } from "./types";

const TOKEN_KEY = "prism-web-token";
const RETRY_MS = 1000;

export async function config(): Promise<{ auth: boolean }> {
  const injected = typeof window !== "undefined"
    ? (window as Window & { __PRISM_WEB__?: { auth: boolean } }).__PRISM_WEB__
    : undefined;
  if (injected) return { auth: injected.auth === true };
  const res = await fetch("/api/config");
  if (!res.ok) throw new Error(`Cannot load web config (${res.status})`);
  const body = (await res.json()) as { auth?: boolean };
  return { auth: body.auth === true };
}

function token(): string | null {
  try {
    return sessionStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
}

function loginRedirect(): void {
  location.assign(`/login.html?next=${encodeURIComponent(location.pathname + location.hash)}`);
}

function rejectToken(): void {
  try {
    sessionStorage.removeItem(TOKEN_KEY);
  } catch {
    // Storage can be disabled by the browser.
  }
  loginRedirect();
}

/** Returns null after redirecting when auth is on but the token is missing. */
async function authHeaders(): Promise<HeadersInit | null> {
  const { auth } = await config();
  const value = token();
  if (auth && !value) {
    loginRedirect();
    return null;
  }
  return auth && value ? { Authorization: `Bearer ${value}` } : {};
}

async function errorMessage(res: Response): Promise<string> {
  const body = await res.json().catch(() => null) as { error?: string } | null;
  return body?.error ?? `Request failed (${res.status})`;
}

export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const auth = await authHeaders();
  if (!auth) throw new Error("Authentication required");
  const res = await fetch("/api/invoke", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...auth },
    body: JSON.stringify({ cmd, args }),
  });
  if (res.status === 401) {
    rejectToken();
    throw new Error("Authentication required");
  }
  if (!res.ok) throw new Error(await errorMessage(res));
  return (await res.json()) as T;
}

function pause(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, RETRY_MS));
}

export async function subscribe(handler: (e: CoreEvent) => void): Promise<() => void> {
  let stopped = false;
  let controller: AbortController | null = null;
  const stop = () => {
    stopped = true;
    controller?.abort();
  };

  const run = async () => {
    while (!stopped) {
      let auth: HeadersInit;
      try {
        const value = await authHeaders();
        if (!value) return;
        auth = value;
      } catch {
        await pause();
        continue;
      }

      controller = new AbortController();
      try {
        const res = await fetch("/api/events", { headers: auth, signal: controller.signal });
        if (res.status === 401) {
          rejectToken();
          return;
        }
        if (!res.ok || !res.body) throw new Error(`Cannot open event stream (${res.status})`);
        const reader = res.body.getReader();
        const decoder = new TextDecoder();
        let buffer = "";
        while (!stopped) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          let end: number;
          while ((end = buffer.indexOf("\n\n")) !== -1) {
            const event = buffer.slice(0, end);
            buffer = buffer.slice(end + 2);
            const line = event.split("\n").find((l) => l.startsWith("data:"));
            const json = line?.slice(5).trim();
            if (json) handler(JSON.parse(json) as CoreEvent);
          }
        }
      } catch {
        // A dropped stream is reopened below.
      }
      if (stopped) return;
      await pause();
    }
  };
  void run();
  return stop;
}

export function subscribeApp(_name: string, _handler: (payload: unknown) => void): () => void {
  return () => {};
}
