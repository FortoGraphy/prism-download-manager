/** Schedule visibility after React has committed the window content.
 * Hidden webviews may suspend animation frames, so a timer must also reveal it.
 * Both paths are cancelled on unmount, including a pending second frame.
 */
export function showAfterPaint(show: () => void): () => void {
  let finished = false;
  let frame: number;
  const reveal = () => {
    if (finished) return;
    finished = true;
    cancelAnimationFrame(frame);
    window.clearTimeout(timer);
    show();
  };
  frame = requestAnimationFrame(() => {
    frame = requestAnimationFrame(reveal);
  });
  const timer = window.setTimeout(reveal, 120);
  return () => {
    finished = true;
    cancelAnimationFrame(frame);
    window.clearTimeout(timer);
  };
}
