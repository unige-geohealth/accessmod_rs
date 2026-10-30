import { describe, expect, it } from "vitest";
import { demoSlug, startProject } from "./start";

const demo = { slug: "demo", info: { demo: true } };
const mine = { slug: "malawi", info: { demo: false } };

describe("start project", () => {
  it("opens the default project when it exists", () => {
    expect(startProject([demo, mine], { default_project: "malawi" })).toBe("malawi");
  });

  it("falls back to the demo project", () => {
    expect(startProject([demo, mine], {})).toBe("demo");
    expect(startProject([demo, mine], { default_project: null })).toBe("demo");
    expect(startProject([demo, mine], { default_project: "removed" })).toBe("demo");
  });

  it("opens nothing without a default or a demo", () => {
    expect(startProject([mine], { default_project: "removed" })).toBeNull();
    expect(startProject([], {})).toBeNull();
  });

  it("finds the demo by its flag, not its slug", () => {
    expect(demoSlug([{ slug: "demo", info: {} }, mine])).toBeNull();
    expect(demoSlug([mine, { slug: "other", info: { demo: true } }])).toBe("other");
  });
});
