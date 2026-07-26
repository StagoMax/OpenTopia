/**
 * Plain-text cleanup for selections copied out of the conversation.
 *
 * Chromium serializes a copied selection block by block, so a drag that stops
 * just past the last glyph of a message still closes that block and opens the
 * next one. The plain-text flavor then carries one or two blank lines the user
 * never selected. The transcript rewrites that flavor through this module so a
 * copied message ends where its text ends.
 */

const leadingBlankLines = /^(?:[ \t\r]*\n)+/;
const trailingBlankLines = /(?:[ \t\r]*\n)*[ \t\r]*$/;

/**
 * Drops blank lines and trailing spaces at both edges of a copied selection
 * while leaving the interior — including the blank lines between paragraphs —
 * exactly as the message wrote it.
 */
export function normalizeCopiedText(value: string): string {
  return value.replace(leadingBlankLines, "").replace(trailingBlankLines, "");
}
