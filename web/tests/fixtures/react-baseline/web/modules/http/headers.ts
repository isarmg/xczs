/** Parse one canonical non-negative integer HTTP header value without accepting
 * signs, whitespace, leading zeroes, fractions, exponents or unsafe integers.
 */
export function parseUnsignedHeader(value: string | null): number | null {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]*)$/.test(value)) {
    return null;
  }
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : null;
}
