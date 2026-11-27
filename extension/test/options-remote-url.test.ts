import { beforeEach, describe, expect, it, vi } from 'vitest';

type El = {
  value: string;
  checked: boolean;
  innerHTML: string;
  textContent: string;
  className: string;
  addEventListener: (type: string, fn: (e: Event) => void) => void;
};
const nodes = new Map<string, El>();
const node = (id: string): El => {
  let el = nodes.get(id);
  if (!el) {
    el = { value: '', checked: false, innerHTML: '', textContent: '', className: '', addEventListener: () => {} };
    nodes.set(id, el);
  }
  return el;
};
let submit: (e: Event) => void | Promise<void> = () => {};
node('form').addEventListener = (type, fn) => {
  if (type === 'submit') submit = fn;
};
(globalThis as unknown as { document: unknown }).document = { getElementById: node };
const sendMessage = vi.fn(async (_msg: unknown) => undefined);
(globalThis as unknown as { chrome: unknown }).chrome = {
  runtime: { sendMessage, openOptionsPage: vi.fn() },
  storage: { local: { get: vi.fn(async () => ({})) } },
};

await import('../src/options/options');

const fire = async (): Promise<void> => {
  await submit({ preventDefault() {} } as Event);
};

beforeEach(() => {
  sendMessage.mockClear();
  node('remoteUrl').value = '';
  node('status').textContent = '';
  node('status').className = '';
});

describe('Options server URL validation', () => {
  it('rejects an invalid non-empty URL without saving', async () => {
    node('remoteUrl').value = 'javascript:alert(1)';
    await fire();
    expect(sendMessage).not.toHaveBeenCalled();
    expect(node('status').textContent).toBe('Enter an http(s) server root URL without credentials');
    expect(node('status').className).toContain('bad');
  });

  it('rejects a URL with credentials', async () => {
    node('remoteUrl').value = 'http://user:pass@127.0.0.1:18186';
    await fire();
    expect(sendMessage).not.toHaveBeenCalled();
    expect(node('status').textContent).toBe('Enter an http(s) server root URL without credentials');
  });

  it('keeps blank valid for native messaging', async () => {
    node('remoteUrl').value = '  ';
    await fire();
    expect(sendMessage).toHaveBeenCalledWith(expect.objectContaining({ kind: 'saveSettings', remoteUrl: '  ' }));
  });

  it('passes a valid server root through to saveSettings', async () => {
    node('remoteUrl').value = 'http://127.0.0.1:18186/base/';
    await fire();
    expect(sendMessage).toHaveBeenCalledWith(expect.objectContaining({ kind: 'saveSettings', remoteUrl: 'http://127.0.0.1:18186/base/' }));
  });
});
