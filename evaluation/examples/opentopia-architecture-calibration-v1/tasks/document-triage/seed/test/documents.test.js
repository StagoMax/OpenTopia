import assert from "node:assert/strict";
import test from "node:test";
import { classifyDocument, extractInvoice } from "../src/documents.js";

test("classifies and extracts invoices", () => {
  const text = "Invoice Number: X-1\nVAT 1.20\nTotal 11.20\nAmount Due 5.00";
  assert.equal(classifyDocument(text), "invoice");
  assert.deepEqual(extractInvoice(text), { invoiceNumber: "X-1", totalCents: 1120, taxCents: 120 });
});

test("does not classify notes by a total alone", () => {
  assert.equal(classifyDocument("Total people: 8"), "other");
});
