import { describe, expect, it } from "vitest";
import { createStore, history, withHistory } from "./store";

type Message = { type: "add"; n: number } | { type: "noop" };
const update = (s: number, m: Message) => (m.type === "add" ? s + m.n : s);

describe("store", () => {
  it("notifies only on change", () => {
    const store = createStore(0, update);
    let calls = 0;
    store.subscribe(() => {
      calls += 1;
    });
    store.dispatch({ type: "noop" });
    store.dispatch({ type: "add", n: 2 });
    expect(store.getState()).toBe(2);
    expect(calls).toBe(1);
  });

  it("undoes and redoes", () => {
    const step = withHistory(update);
    let h = history(0);
    h = step(h, { type: "do", message: { type: "add", n: 1 } });
    h = step(h, { type: "do", message: { type: "add", n: 2 } });
    expect(h.present).toBe(3);
    h = step(h, { type: "undo" });
    expect(h.present).toBe(1);
    h = step(h, { type: "redo" });
    expect(h.present).toBe(3);
    h = step(h, { type: "undo" });
    h = step(h, { type: "do", message: { type: "add", n: 10 } });
    expect(h.future).toEqual([]);
    expect(step(h, { type: "redo" })).toBe(h);
  });
});
