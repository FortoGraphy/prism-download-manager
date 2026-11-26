const FULLSCREEN_MESSAGE = 'prism-video-fullscreen';

/** Tracks fullscreen in this document and in ancestor documents across frame origins. */
export class FullscreenState {
  private parentActive = false;

  constructor(private readonly changed: () => void) {}

  get active(): boolean {
    return this.parentActive || document.fullscreenElement !== null;
  }

  start(): void {
    document.addEventListener('fullscreenchange', this.publish);
    window.addEventListener('message', this.onMessage);
    if (window.parent !== window) {
      window.parent.postMessage({ type: FULLSCREEN_MESSAGE, request: true }, '*');
    }
    this.publish();
  }

  stop(): void {
    document.removeEventListener('fullscreenchange', this.publish);
    window.removeEventListener('message', this.onMessage);
    this.parentActive = false;
  }

  private send(frame: Window): void {
    // A visibility hint only. It does not grant access to any extension actions or data.
    frame.postMessage({ type: FULLSCREEN_MESSAGE, active: this.active }, '*');
  }

  private publish = (): void => {
    this.changed();
    for (let i = 0; i < window.frames.length; i++) this.send(window.frames[i]!);
  };

  private onMessage = ({ data, source }: MessageEvent): void => {
    if (data?.type !== FULLSCREEN_MESSAGE) return;
    if (window.parent !== window && source === window.parent && typeof data.active === 'boolean') {
      if (this.parentActive === data.active) return;
      this.parentActive = data.active;
      this.publish();
    } else if (data.request === true) {
      for (let i = 0; i < window.frames.length; i++) {
        const frame = window.frames[i]!;
        if (frame === source) this.send(frame);
      }
    }
  };
}
