export function collapseDisplayWhitespace(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}
