import { api } from '../shared/api';
import type { ConnectionInfo } from '../shared/messages';

const action = api.action;

/** Per-tab media count (tab-specific text overrides the global "!" badge). */
export function setTabCount(tabId: number, count: number): void {
  if (tabId < 0) return;
  const text = count > 0 ? (count > 99 ? '99+' : String(count)) : null;
  action.setBadgeText({ tabId, text: text as string }).catch(() => {});
  if (text) action.setBadgeBackgroundColor({ tabId, color: '#5b6cff' }).catch(() => {});
}

/** Global badge: "!" while the app/host is unreachable after a failed hand-off. */
export function showUnreachable(on: boolean, title?: string): void {
  action.setBadgeText({ text: on ? '!' : '' }).catch(() => {});
  if (on) action.setBadgeBackgroundColor({ color: '#e5484d' }).catch(() => {});
  action.setTitle({ title: on ? `Prism — ${title ?? 'not reachable'}` : 'Prism' }).catch(() => {});
}

export function reflectConnection(c: ConnectionInfo): void {
  if (c.state === 'connected') showUnreachable(false);
  else if (c.state === 'host_missing') showUnreachable(true, 'browser integration not installed');
  else if (c.state === 'app_down') {
    showUnreachable(true, c.errorCode === 'unauthorized' ? 'check Prism web token in Options' : 'server or app not reachable');
  }
}
