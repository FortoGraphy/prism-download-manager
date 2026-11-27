import { describe, expect, it } from "vitest";
import { parseRoute } from "./route";

describe("parseRoute", () => {
  it("falls back to main on malformed percent escapes", () => {
    expect(parseRoute("#/batch/%")).toEqual({ name: "main" });
    expect(parseRoute("#/complete/%E0%A4%A")).toEqual({ name: "main" });
  });

  it("still decodes valid ids", () => {
    expect(parseRoute("#/batch/hello%20world")).toEqual({ name: "batch", id: "hello world" });
  });
});
