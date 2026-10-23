import { describe, expect, it } from "vitest";
import catalog from "../i18n/catalog.json";

describe("catalogue", () => {
  it("has every key in every language", () => {
    const keys = Object.keys(catalog.messages.en).sort();
    for (const language of catalog.languages) {
      const messages: Record<string, string> =
        catalog.messages[language as keyof typeof catalog.messages];
      expect(Object.keys(messages).sort()).toEqual(keys);
      for (const key of keys) expect(messages[key], `${language} ${key}`).not.toBe("");
    }
  });
});
