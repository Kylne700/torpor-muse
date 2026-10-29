'use strict';
// Money rules of the clinic desk. All amounts are in shekels, rounded to agorot.

function roundMoney(n) {
  return Math.round(n * 100) / 100;
}

function priceOf(code, tariffs = DEFAULT_TARIFFS) {
  const row = tariffs.find((t) => t.code === code);
  return row ? Number(row.price) : null;
}

function lineTotal(line, tariffs) {
  const price = priceOf(line.code, tariffs);
  if (price === null) throw new Error('no tariff for ' + line.code);
  return roundMoney(price * (line.qty || 1));
}

function invoiceTotal(invoice, tariffs) {
  const sum = (invoice.lines || []).reduce((acc, l) => acc + lineTotal(l, tariffs), 0);
  const covered = invoice.insurerShare ? roundMoney((sum * invoice.insurerShare) / 100) : 0;
  return roundMoney(sum - covered);
}

function invoiceNumber(id, date = new Date()) {
  return date.getFullYear() + '-' + String(id).padStart(5, '0');
}

const DEFAULT_TARIFFS = [
  { code: 'T100', name: 'General visit', price: 60 },
  { code: 'T101', name: 'Follow-up visit', price: 97 },
  { code: 'T102', name: 'Blood panel', price: 134 },
  { code: 'T103', name: 'X-ray', price: 171 },
  { code: 'T104', name: 'Ultrasound', price: 208 },
  { code: 'T105', name: 'ECG', price: 245 },
  { code: 'T106', name: 'Vaccination', price: 282 },
  { code: 'T107', name: 'Wound care', price: 319 },
  { code: 'T108', name: 'Physio session', price: 356 },
  { code: 'T109', name: 'Eye test', price: 393 },
  { code: 'T110', name: 'Hearing test', price: 90 },
  { code: 'T111', name: 'Allergy test', price: 127 },
  { code: 'T112', name: 'General visit (2)', price: 164 },
  { code: 'T113', name: 'Follow-up visit (2)', price: 201 },
  { code: 'T114', name: 'Blood panel (2)', price: 238 },
  { code: 'T115', name: 'X-ray (2)', price: 275 },
  { code: 'T116', name: 'Ultrasound (2)', price: 312 },
  { code: 'T117', name: 'ECG (2)', price: 349 },
  { code: 'T118', name: 'Vaccination (2)', price: 386 },
  { code: 'T119', name: 'Wound care (2)', price: 83 },
  { code: 'T120', name: 'Physio session (2)', price: 120 },
  { code: 'T121', name: 'Eye test (2)', price: 157 },
  { code: 'T122', name: 'Hearing test (2)', price: 194 },
  { code: 'T123', name: 'Allergy test (2)', price: 231 },
  { code: 'T124', name: 'General visit (3)', price: 268 },
  { code: 'T125', name: 'Follow-up visit (3)', price: 305 },
  { code: 'T126', name: 'Blood panel (3)', price: 342 },
  { code: 'T127', name: 'X-ray (3)', price: 379 },
  { code: 'T128', name: 'Ultrasound (3)', price: 76 },
  { code: 'T129', name: 'ECG (3)', price: 113 },
  { code: 'T130', name: 'Vaccination (3)', price: 150 },
  { code: 'T131', name: 'Wound care (3)', price: 187 },
  { code: 'T132', name: 'Physio session (3)', price: 224 },
  { code: 'T133', name: 'Eye test (3)', price: 261 },
  { code: 'T134', name: 'Hearing test (3)', price: 298 },
  { code: 'T135', name: 'Allergy test (3)', price: 335 },
  { code: 'T136', name: 'General visit (4)', price: 372 },
  { code: 'T137', name: 'Follow-up visit (4)', price: 69 },
  { code: 'T138', name: 'Blood panel (4)', price: 106 },
  { code: 'T139', name: 'X-ray (4)', price: 143 },
  { code: 'T140', name: 'Ultrasound (4)', price: 180 },
  { code: 'T141', name: 'ECG (4)', price: 217 },
  { code: 'T142', name: 'Vaccination (4)', price: 254 },
  { code: 'T143', name: 'Wound care (4)', price: 291 },
  { code: 'T144', name: 'Physio session (4)', price: 328 },
  { code: 'T145', name: 'Eye test (4)', price: 365 },
  { code: 'T146', name: 'Hearing test (4)', price: 62 },
  { code: 'T147', name: 'Allergy test (4)', price: 99 },
];

module.exports = { DEFAULT_TARIFFS, roundMoney, priceOf, lineTotal, invoiceTotal, invoiceNumber };
