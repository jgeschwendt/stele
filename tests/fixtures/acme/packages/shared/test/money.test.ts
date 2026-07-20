import { money, toCents, type Money } from "../src/money";

describe("money", () => {
  it("constructs integer-cent values", () => {
    const m: Money = money(1299, "usd");
    expect(m.cents).toBe(1299);
    expect(m.currency).toBe("usd");
  });

  it("rejects non-integer cents", () => {
    expect(() => money(12.99)).toThrow(TypeError);
  });

  it("round-trips through toCents", () => {
    expect(toCents(money(500))).toBe(500);
  });
});
