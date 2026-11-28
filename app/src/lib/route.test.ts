import { describe, expect, it } from "vitest";
import { parseRoute, routeKey } from "./route";

describe("parseRoute", () => {
  it("falls back to main on malformed percent escapes", () => {
    expect(parseRoute("#/batch/%")).toEqual({ name: "main" });
    expect(parseRoute("#/complete/%E0%A4%A")).toEqual({ name: "main" });
  });

  it("still decodes valid ids", () => {
    expect(parseRoute("#/batch/hello%20world")).toEqual({ name: "batch", id: "hello world" });
  });
});

describe("routeKey", () => {
  it("changes when a tools request changes but the route name stays the same", () => {
    const first: Parameters<typeof routeKey>[0] = { name: "tools", request: { tab: "convert" } };
    const second: Parameters<typeof routeKey>[0] = { name: "tools", request: { tab: "shrink" } };
    expect(routeKey(first)).not.toBe(routeKey(second));
    expect(routeKey(parseRoute("#/tools/convert"))).toBe(routeKey(first));
  });
});
