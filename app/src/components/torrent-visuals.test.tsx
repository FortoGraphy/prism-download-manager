import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { PeerLanes, PieceMap } from "./Visuals";

// #176: a torrent shows its pieces and peers instead of segments and connections.
describe("torrent visuals", () => {
  it("draws one lane per peer, the fastest full, with its client or address", () => {
    const html = renderToStaticMarkup(
      <PeerLanes
        max={2}
        peers={[
          { address: "10.0.0.1:6881", client: "qBittorrent 5.1.2", downloaded: 9000, speed: 2048 },
          { address: "10.0.0.2:51413", client: null, downloaded: 500, speed: 1024 },
          { address: "10.0.0.3:6881", client: "Transmission 4.0", downloaded: 0, speed: 0 },
        ]}
      />,
    );
    expect(html.match(/class="lane lane--peer"/g)).toHaveLength(2);
    expect(html).toContain("qBittorrent 5.1.2");
    expect(html).toContain("10.0.0.2:51413");
    expect(html).not.toContain("Transmission");
    expect(html).toContain("scaleX(1)");
    expect(html).toContain("scaleX(0.5)");
  });

  it("says so when no peer is connected", () => {
    expect(renderToStaticMarkup(<PeerLanes peers={[]} />)).toContain("No peers connected yet.");
  });

  it("draws the downloaded runs of pieces, neighbours at the same level as one", () => {
    const html = renderToStaticMarkup(<PieceMap pieces={{ total: 8, done: 5, map: [1, 1, 0, 0.5, 0.55, 0, 0, 1] }} />);
    const runs = [...html.matchAll(/--a:([\d.]+);--b:([\d.]+);--o:([\d.]+)/g)].map((m) => m.slice(1).map(Number));
    expect(runs).toEqual([
      [0, 0.25, 1],
      [0.375, 0.625, 0.3 + 0.7 * 0.5],
      [0.875, 1, 1],
    ]);
    expect(html).toContain("Pieces 5 / 8");
  });

  it("draws an empty bar before the pieces are known", () => {
    expect(renderToStaticMarkup(<PieceMap pieces={null} />)).toBe('<div class="segbar"></div>');
  });
});
