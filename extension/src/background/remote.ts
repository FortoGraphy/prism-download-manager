import { api } from '../shared/api';
import { NativeError } from '../shared/correlator';
import type { AppRequest, ReplyMap } from '../shared/protocol';
import { STATE_KEY, normalizeState } from '../shared/settings';
import type { RequestOptions } from './native';

type AnyReply = {
  id?: string | number;
  ok?: boolean;
  error?: { code?: string; message?: string } | string;
} & Record<string, unknown>;

let counter = 0;
const prefix = Math.random().toString(36).slice(2, 8);

/** Reads the remote settings from the same persisted state the Options page writes. */
export async function getRemoteConfig(): Promise<{ url: string; token: string }> {
  try {
    const r = await api.storage.local.get(STATE_KEY);
    const st = normalizeState(r?.[STATE_KEY]);
    return { url: st.remoteUrl, token: st.remoteToken };
  } catch {
    return { url: '', token: '' };
  }
}

/** Direct HTTP transport for a prism-web server; same envelope and reply shape as the native host. */
export class RemoteClient {
  constructor(
    private readonly baseUrl: string,
    private readonly token: string,
  ) {}

  async request<K extends AppRequest['type']>(
    msg: Extract<AppRequest, { type: K }>,
    opts: RequestOptions = {},
  ): Promise<ReplyMap[K]> {
    const timeoutMs = opts.timeoutMs ?? 3000;
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    const id = `${prefix}-${++counter}`;
    try {
      let res: Response;
      try {
        res = await fetch(`${this.baseUrl}/v1/message`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${this.token}` },
          body: JSON.stringify({ ...msg, id, ...(opts.noLaunch ? { noLaunch: true } : {}) }),
          signal: controller.signal,
        });
      } catch {
        if (controller.signal.aborted) throw new NativeError('timeout', `no reply within ${timeoutMs} ms`);
        throw new NativeError('app_unreachable', 'Prism web server unreachable');
      }
      if (res.status === 401 || res.status === 403) {
        throw new NativeError('unauthorized', 'Prism web server rejected the token');
      }
      if (!res.ok) throw new NativeError('app_unreachable', `Prism web server returned ${res.status}`);
      let reply: AnyReply;
      try {
        reply = (await res.json()) as AnyReply;
      } catch {
        throw new NativeError('error', 'Prism web server sent an invalid reply');
      }
      if (reply?.ok === false) {
        const e = reply.error;
        throw new NativeError(
          typeof e === 'string' ? e : e?.code ?? 'app_error',
          typeof e === 'string' ? e : e?.message ?? 'request failed',
        );
      }
      return reply as unknown as ReplyMap[K];
    } finally {
      clearTimeout(timer);
    }
  }
}
