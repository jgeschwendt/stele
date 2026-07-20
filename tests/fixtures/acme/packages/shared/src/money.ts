// Money — integer-cents value type shared across the acme monorepo.
// See adr/0007: all currency is integer cents; floats never represent money.
// stele:landmark money-type
export type Money = {
  readonly cents: number;
  readonly currency: string;
};

export function money(cents: number, currency = "usd"): Money {
  if (!Number.isInteger(cents)) {
    throw new TypeError(`money() requires integer cents, got ${cents}`);
  }
  return { cents, currency };
}

export function toCents(m: Money): number {
  return m.cents;
}
