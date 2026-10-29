import { api } from '../shared/api';
import { Correlator, NativeError, classifyDisconnect } from '../shared/correlator';
import type { AppRequest, ReplyMap } from '../shared/protocol';
import { NATIVE_HOST_NAME } from '../shared/protocol';

type AnyReply = { id?: string | number; ok?: boolean; error?: { code?: string; message?: string } } & Record<string, unknown>;

export interface RequestOptions {
  timeoutMs?: number;
  /** Ask the host not to launch the app when it isn't running. */
  noLaunch?: boolean;
}

type Listener = (ok: boolean, err?: NativeError, reply?: AnyReply) => void;

/**
 * Long-lived native messaging port with request/response correlation.
 *
 * - Connects lazily on the first request and keeps the port open (this also
 *   keeps the Chrome MV3 service worker alive, Chrome 105+).
 * - Reconnects on the next request after a disconnect, with backoff when the
 *   host is missing so we don't spawn processes in a loop.
 * - Falls back to one-shot `sendNativeMessage` when the port dies mid-request.
 */
export class NativeClient {
  private port: chrome.runtime.Port | null = null;
  private readonly corr = new Correlator<AnyReply>(3000);
  private hostMissingUntil = 0;
  private lastMissingError: NativeError | null = null;
  private listeners: Listener[] = [];

  constructor(private readonly hostName = NATIVE_HOST_NAME) {}

  /** Observes every completed request (used for the connection indicator). */
  onResult(l: Listener): void {
    this.listeners.push(l);
  }

  get connected(): boolean {
    return this.port !== null;
  }

  /** Eagerly opens the port if not already connected (called when service worker wakes). */
  ensureConnected(): void {
    if (!this.port && Date.now() >= this.hostMissingUntil) {
      try {
        this.connect();
      } catch {
        /* ignore */
      }
    }
  }

  private emit(ok: boolean, err?: NativeError, reply?: AnyReply): void {
    for (const l of this.listeners) {
      try {
        l(ok, err, reply);
      } catch {
        /* ignore */
      }
    }
  }

  private connect(): chrome.runtime.Port {
    if (this.port) return this.port;
    const port = api.runtime.connectNative(this.hostName);
    this.port = port;
    port.onMessage.addListener((msg: AnyReply) => {
      if (!this.corr.resolve(msg)) console.debug('[prism] unmatched native reply', msg);
    });
    port.onDisconnect.addListener((p: chrome.runtime.Port & { error?: { message?: string } }) => {
      const message = p.error?.message ?? api.runtime.lastError?.message ?? 'native host disconnected';
      const code = classifyDisconnect(message);
      if (this.port === port) this.port = null;
      if (code === 'host_missing' || code === 'host_forbidden') {
        this.hostMissingUntil = Date.now() + 5000;
        this.lastMissingError = new NativeError(code, message);
      }
      this.corr.rejectAll(new NativeError(code, message));
    });
    return port;
  }

  /** Closes the port (the host exits on EOF). */
  disconnect(): void {
    const p = this.port;
    this.port = null;
    try {
      p?.disconnect();
    } catch {
      /* ignore */
    }
    this.corr.rejectAll(new NativeError('disconnected', 'port closed'));
  }

  async request<K extends AppRequest['type']>(
    msg: Extract<AppRequest, { type: K }>,
    opts: RequestOptions = {},
  ): Promise<ReplyMap[K]> {
    const timeoutMs = opts.timeoutMs ?? 3000;
    const id = this.corr.nextId();
    const envelope = { ...msg, id, ...(opts.noLaunch ? { noLaunch: true } : {}) };
    try {
      if (Date.now() < this.hostMissingUntil && this.lastMissingError) throw this.lastMissingError;
      let reply: AnyReply;
      try {
        reply = await this.viaPort(id, envelope, timeoutMs);
      } catch (e) {
        const code = e instanceof NativeError ? e.code : 'disconnected';
        if (code !== 'host_exited' && code !== 'disconnected') throw e;
        // The port died under us: retry once as a one-shot message.
        reply = await this.viaOneShot(envelope, timeoutMs);
      }
      if (reply.ok === false) {
        throw new NativeError(reply.error?.code ?? 'app_error', reply.error?.message ?? 'request failed');
      }
      this.emit(true, undefined, reply);
      return reply as unknown as ReplyMap[K];
    } catch (e) {
      const err = e instanceof NativeError ? e : new NativeError('error', String((e as Error)?.message ?? e));
      this.emit(false, err);
      throw err;
    }
  }

  private viaPort(id: string, envelope: object, timeoutMs: number): Promise<AnyReply> {
    const port = this.connect();
    const wait = this.corr.wait(id, timeoutMs);
    try {
      port.postMessage(envelope);
    } catch (e) {
      if (this.port === port) this.port = null;
      this.corr.reject(id, new NativeError('disconnected', String((e as Error)?.message ?? e)));
    }
    return wait;
  }

  private async viaOneShot(envelope: object, timeoutMs: number): Promise<AnyReply> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<never>((_, rej) => {
      timer = setTimeout(() => rej(new NativeError('timeout', `no reply within ${timeoutMs} ms`)), timeoutMs);
    });
    try {
      const send = api.runtime.sendNativeMessage(this.hostName, envelope) as Promise<AnyReply>;
      return await Promise.race([send, timeout]);
    } catch (e) {
      if (e instanceof NativeError) throw e;
      const message = String((e as Error)?.message ?? e);
      throw new NativeError(classifyDisconnect(message), message);
    } finally {
      clearTimeout(timer);
    }
  }
}

export const native = new NativeClient();
