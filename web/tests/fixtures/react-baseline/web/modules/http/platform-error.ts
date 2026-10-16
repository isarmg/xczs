import { isErrorEnvelope } from "../../platform.ts";

/** Decode only the current platform error contract, never product error aliases.
 */
export function platformErrorCode(
  text: string,
  contentType: string | null,
): string | null {
  if (contentType?.split(";", 1)[0].trim().toLowerCase() !== "application/json")
    return null;
  try {
    const value = JSON.parse(text);
    return isErrorEnvelope(value) ? value.code : null;
  } catch {
    return null;
  }
}
