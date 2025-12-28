/**
 * Truncate a string to a maximum length, adding ellipsis if truncated
 */
export function truncate(text: string, maxLength: number): string {
  if (text.length <= maxLength) {
    return text;
  }
  if (maxLength <= 3) {
    return text.slice(0, maxLength);
  }
  return text.slice(0, maxLength - 1) + '…';
}
