'use strict';
// Monthly numbers for the clinic manager.

function monthOf(ms) {
  return new Date(ms).toISOString().slice(0, 7);
}

function monthlyVisits(visits, appointments) {
  const out = {};
  for (const v of visits) {
    const m = monthOf(v.createdAt);
    out[m] = out[m] || { visits: 0, booked: 0, cancelled: 0 };
    out[m].visits++;
  }
  for (const a of appointments) {
    const m = monthOf(a.start);
    out[m] = out[m] || { visits: 0, booked: 0, cancelled: 0 };
    if (a.status === 'cancelled') out[m].cancelled++;
    else out[m].booked++;
  }
  return out;
}

function busiestWeekday(appointments) {
  const counts = [0, 0, 0, 0, 0, 0, 0];
  for (const a of appointments) counts[new Date(a.start).getDay()]++;
  const best = counts.indexOf(Math.max(...counts));
  return { weekday: best, count: counts[best] };
}

function noShowRate(appointments) {
  const past = appointments.filter((a) => a.start < Date.now() && a.status !== 'cancelled');
  if (past.length === 0) return 0;
  return Math.round((past.filter((a) => a.noShow).length / past.length) * 1000) / 10;
}

function revenueByInsurer(invoices, insurers) {
  const names = new Map(insurers.map((i) => [i.id, i.name]));
  const out = {};
  for (const inv of invoices) {
    const name = names.get(inv.insurerId) || 'self-pay';
    out[name] = (out[name] || 0) + Number(inv.total || 0);
  }
  return out;
}

function topDiagnoses(diagnoses, n = 5) {
  const counts = new Map();
  for (const d of diagnoses) counts.set(d.code, (counts.get(d.code) || 0) + 1);
  return [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, n).map(([code, count]) => ({ code, count }));
}

function ageGroups(patients, now = new Date()) {
  const groups = { child: 0, adult: 0, senior: 0 };
  for (const p of patients) {
    if (!p.born) continue;
    const age = now.getFullYear() - Number(String(p.born).slice(0, 4));
    if (age < 18) groups.child++;
    else if (age < 65) groups.adult++;
    else groups.senior++;
  }
  return groups;
}

module.exports = { monthlyVisits, busiestWeekday, noShowRate, revenueByInsurer, topDiagnoses, ageGroups };
