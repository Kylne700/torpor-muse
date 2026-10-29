'use strict';
// Clinic desk API: patients, doctors, appointments and the paperwork around them.
const express = require('express');
const billing = require('./lib/billing');
const reports = require('./lib/reports');

const app = express();
app.use(express.json());

const PORT = Number(process.env.PORT) || 6000;
const PAGE_SIZE = 25;
const SLOT_MINUTES = 20;

const db = {
  patients: [
    { id: 1, name: 'Dana Mizrahi', phone: '052-555-0101', born: '1984-02-11', createdAt: 1700000000000 },
    { id: 2, name: 'Omer Katz', phone: '054-555-0102', born: '1991-07-30', createdAt: 1700000100000 },
  ],
  doctors: [
    { id: 1, name: 'Dr. Levin', specialty: 'family', phone: '03-555-0110', createdAt: 1700000000000 },
  ],
  appointments: [],
  rooms: [], invoices: [], prescriptions: [], labs: [], visits: [], insurers: [], nurses: [], wards: [],
  supplies: [], vaccines: [], referrals: [], allergies: [], devices: [], shifts: [], diagnoses: [],
  therapies: [], pharmacies: [], transports: [], donors: [], archives: [], tariffs: [], waitlist: [],
  complaints: [], rotas: [],
};

function nextId(rows) {
  return rows.reduce((max, r) => Math.max(max, r.id), 0) + 1;
}

function parseDate(text) {
  const t = Date.parse(text);
  return Number.isNaN(t) ? null : t;
}

// Patient list. ?phone= finds a patient by the number they gave at the desk; ?name= searches by name.
app.get('/api/patients', (req, res) => {
  const digits = (s) => String(s || '').replace(/\D/g, '');
  let rows = db.patients;
  if (req.query.phone) rows = rows.filter((p) => digits(p.phone).endsWith(digits(req.query.phone)));
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((p) => p.name.toLowerCase().includes(q));
  }
  res.json({ patients: rows.slice(0, PAGE_SIZE), count: rows.length });
});

app.post('/api/patients', (req, res) => {
  const { name, phone, born } = req.body || {};
  if (!name || !phone) return res.status(400).json({ error: 'name and phone are required' });
  const patient = { id: nextId(db.patients), name, phone, born: born || null, createdAt: Date.now() };
  db.patients.push(patient);
  res.status(201).json(patient);
});

app.get('/api/patients/:id', (req, res) => {
  const patient = db.patients.find((p) => p.id === Number(req.params.id));
  if (!patient) return res.status(404).json({ error: 'patient not found' });
  res.json({ ...patient, appointments: db.appointments.filter((a) => a.patientId === patient.id) });
});

app.get('/api/appointments', (req, res) => {
  const day = req.query.day ? parseDate(req.query.day) : null;
  let rows = db.appointments;
  if (req.query.doctorId) rows = rows.filter((a) => a.doctorId === Number(req.query.doctorId));
  if (day) rows = rows.filter((a) => a.start >= day && a.start < day + 24 * 3600 * 1000);
  res.json({ appointments: rows.sort((a, b) => a.start - b.start), count: rows.length });
});

const taken = new Set();

function slotKey(doctorId, start) {
  return doctorId + '@' + start;
}

function takeSlot(doctorId, start) {
  const key = slotKey(doctorId, start);
  if (taken.has(key)) return false;
  taken.add(key);
  return true;
}

// Books a patient with a doctor at a start time; the start must fall on a ${SLOT_MINUTES}-minute boundary.
app.post('/api/appointments', (req, res) => {
  const { patientId, doctorId, start } = req.body || {};
  const when = parseDate(start);
  if (!when) return res.status(400).json({ error: 'start must be a date' });
  if (when % (SLOT_MINUTES * 60 * 1000) !== 0) {
    return res.status(400).json({ error: 'appointments start every ' + SLOT_MINUTES + ' minutes' });
  }
  if (!db.patients.some((p) => p.id === Number(patientId))) return res.status(404).json({ error: 'patient not found' });
  if (!db.doctors.some((d) => d.id === Number(doctorId))) return res.status(404).json({ error: 'doctor not found' });
  if (!takeSlot(Number(doctorId), when)) return res.status(409).json({ error: 'that time is taken' });
  const appt = { id: nextId(db.appointments), patientId: Number(patientId), doctorId: Number(doctorId), start: when, status: 'booked' };
  db.appointments.push(appt);
  res.status(201).json(appt);
});

// ---- doctors ----
app.get('/api/doctors', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.doctors;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ doctors: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/doctors/:id', (req, res) => {
  const doctor = db.doctors.find((r) => r.id === Number(req.params.id));
  if (!doctor) return res.status(404).json({ error: 'doctor not found' });
  res.json(doctor);
});

app.post('/api/doctors', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.specialty) return res.status(400).json({ error: 'specialty is required' });
  if (!body.phone) return res.status(400).json({ error: 'phone is required' });
  const doctor = { id: nextId(db.doctors), name: body.name, specialty: body.specialty, phone: body.phone, createdAt: Date.now() };
  db.doctors.push(doctor);
  res.status(201).json(doctor);
});

app.put('/api/doctors/:id', (req, res) => {
  const doctor = db.doctors.find((r) => r.id === Number(req.params.id));
  if (!doctor) return res.status(404).json({ error: 'doctor not found' });
  Object.assign(doctor, req.body || {}, { id: doctor.id });
  res.json(doctor);
});

app.delete('/api/doctors/:id', (req, res) => {
  const at = db.doctors.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'doctor not found' });
  db.doctors.splice(at, 1);
  res.status(204).end();
});

// ---- rooms ----
app.get('/api/rooms', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.rooms;
  res.json({ rooms: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/rooms/:id', (req, res) => {
  const room = db.rooms.find((r) => r.id === Number(req.params.id));
  if (!room) return res.status(404).json({ error: 'room not found' });
  res.json(room);
});

app.post('/api/rooms', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.floor) return res.status(400).json({ error: 'floor is required' });
  const room = { id: nextId(db.rooms), name: body.name, floor: body.floor, createdAt: Date.now() };
  db.rooms.push(room);
  res.status(201).json(room);
});

app.put('/api/rooms/:id', (req, res) => {
  const room = db.rooms.find((r) => r.id === Number(req.params.id));
  if (!room) return res.status(404).json({ error: 'room not found' });
  Object.assign(room, req.body || {}, { id: room.id });
  res.json(room);
});

app.delete('/api/rooms/:id', (req, res) => {
  const at = db.rooms.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'room not found' });
  db.rooms.splice(at, 1);
  res.status(204).end();
});

// ---- invoices ----
app.get('/api/invoices', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.invoices;
  res.json({ invoices: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/invoices/:id', (req, res) => {
  const invoice = db.invoices.find((r) => r.id === Number(req.params.id));
  if (!invoice) return res.status(404).json({ error: 'invoice not found' });
  res.json(invoice);
});

app.post('/api/invoices', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.lines) return res.status(400).json({ error: 'lines is required' });
  const invoice = { id: nextId(db.invoices), patientId: body.patientId, lines: body.lines, createdAt: Date.now() };
  db.invoices.push(invoice);
  res.status(201).json(invoice);
});

app.put('/api/invoices/:id', (req, res) => {
  const invoice = db.invoices.find((r) => r.id === Number(req.params.id));
  if (!invoice) return res.status(404).json({ error: 'invoice not found' });
  Object.assign(invoice, req.body || {}, { id: invoice.id });
  res.json(invoice);
});

app.delete('/api/invoices/:id', (req, res) => {
  const at = db.invoices.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'invoice not found' });
  db.invoices.splice(at, 1);
  res.status(204).end();
});

// ---- prescriptions ----
app.get('/api/prescriptions', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.prescriptions;
  res.json({ prescriptions: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/prescriptions/:id', (req, res) => {
  const prescription = db.prescriptions.find((r) => r.id === Number(req.params.id));
  if (!prescription) return res.status(404).json({ error: 'prescription not found' });
  res.json(prescription);
});

app.post('/api/prescriptions', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.drug) return res.status(400).json({ error: 'drug is required' });
  if (!body.dose) return res.status(400).json({ error: 'dose is required' });
  const prescription = { id: nextId(db.prescriptions), patientId: body.patientId, drug: body.drug, dose: body.dose, createdAt: Date.now() };
  db.prescriptions.push(prescription);
  res.status(201).json(prescription);
});

app.put('/api/prescriptions/:id', (req, res) => {
  const prescription = db.prescriptions.find((r) => r.id === Number(req.params.id));
  if (!prescription) return res.status(404).json({ error: 'prescription not found' });
  Object.assign(prescription, req.body || {}, { id: prescription.id });
  res.json(prescription);
});

app.delete('/api/prescriptions/:id', (req, res) => {
  const at = db.prescriptions.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'prescription not found' });
  db.prescriptions.splice(at, 1);
  res.status(204).end();
});

// ---- labs ----
app.get('/api/labs', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.labs;
  res.json({ labs: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/labs/:id', (req, res) => {
  const lab = db.labs.find((r) => r.id === Number(req.params.id));
  if (!lab) return res.status(404).json({ error: 'lab not found' });
  res.json(lab);
});

app.post('/api/labs', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.test) return res.status(400).json({ error: 'test is required' });
  const lab = { id: nextId(db.labs), patientId: body.patientId, test: body.test, createdAt: Date.now() };
  db.labs.push(lab);
  res.status(201).json(lab);
});

app.put('/api/labs/:id', (req, res) => {
  const lab = db.labs.find((r) => r.id === Number(req.params.id));
  if (!lab) return res.status(404).json({ error: 'lab not found' });
  Object.assign(lab, req.body || {}, { id: lab.id });
  res.json(lab);
});

app.delete('/api/labs/:id', (req, res) => {
  const at = db.labs.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'lab not found' });
  db.labs.splice(at, 1);
  res.status(204).end();
});

// ---- visits ----
app.get('/api/visits', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.visits;
  res.json({ visits: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/visits/:id', (req, res) => {
  const visit = db.visits.find((r) => r.id === Number(req.params.id));
  if (!visit) return res.status(404).json({ error: 'visit not found' });
  res.json(visit);
});

app.post('/api/visits', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.doctorId) return res.status(400).json({ error: 'doctorId is required' });
  if (!body.notes) return res.status(400).json({ error: 'notes is required' });
  const visit = { id: nextId(db.visits), patientId: body.patientId, doctorId: body.doctorId, notes: body.notes, createdAt: Date.now() };
  db.visits.push(visit);
  res.status(201).json(visit);
});

app.put('/api/visits/:id', (req, res) => {
  const visit = db.visits.find((r) => r.id === Number(req.params.id));
  if (!visit) return res.status(404).json({ error: 'visit not found' });
  Object.assign(visit, req.body || {}, { id: visit.id });
  res.json(visit);
});

app.delete('/api/visits/:id', (req, res) => {
  const at = db.visits.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'visit not found' });
  db.visits.splice(at, 1);
  res.status(204).end();
});

// ---- insurers ----
app.get('/api/insurers', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.insurers;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ insurers: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/insurers/:id', (req, res) => {
  const insurer = db.insurers.find((r) => r.id === Number(req.params.id));
  if (!insurer) return res.status(404).json({ error: 'insurer not found' });
  res.json(insurer);
});

app.post('/api/insurers', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.plan) return res.status(400).json({ error: 'plan is required' });
  const insurer = { id: nextId(db.insurers), name: body.name, plan: body.plan, createdAt: Date.now() };
  db.insurers.push(insurer);
  res.status(201).json(insurer);
});

app.put('/api/insurers/:id', (req, res) => {
  const insurer = db.insurers.find((r) => r.id === Number(req.params.id));
  if (!insurer) return res.status(404).json({ error: 'insurer not found' });
  Object.assign(insurer, req.body || {}, { id: insurer.id });
  res.json(insurer);
});

app.delete('/api/insurers/:id', (req, res) => {
  const at = db.insurers.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'insurer not found' });
  db.insurers.splice(at, 1);
  res.status(204).end();
});

// ---- nurses ----
app.get('/api/nurses', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.nurses;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ nurses: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/nurses/:id', (req, res) => {
  const nurse = db.nurses.find((r) => r.id === Number(req.params.id));
  if (!nurse) return res.status(404).json({ error: 'nurse not found' });
  res.json(nurse);
});

app.post('/api/nurses', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.ward) return res.status(400).json({ error: 'ward is required' });
  if (!body.phone) return res.status(400).json({ error: 'phone is required' });
  const nurse = { id: nextId(db.nurses), name: body.name, ward: body.ward, phone: body.phone, createdAt: Date.now() };
  db.nurses.push(nurse);
  res.status(201).json(nurse);
});

app.put('/api/nurses/:id', (req, res) => {
  const nurse = db.nurses.find((r) => r.id === Number(req.params.id));
  if (!nurse) return res.status(404).json({ error: 'nurse not found' });
  Object.assign(nurse, req.body || {}, { id: nurse.id });
  res.json(nurse);
});

app.delete('/api/nurses/:id', (req, res) => {
  const at = db.nurses.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'nurse not found' });
  db.nurses.splice(at, 1);
  res.status(204).end();
});

// ---- wards ----
app.get('/api/wards', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.wards;
  res.json({ wards: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/wards/:id', (req, res) => {
  const ward = db.wards.find((r) => r.id === Number(req.params.id));
  if (!ward) return res.status(404).json({ error: 'ward not found' });
  res.json(ward);
});

app.post('/api/wards', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.beds) return res.status(400).json({ error: 'beds is required' });
  const ward = { id: nextId(db.wards), name: body.name, beds: body.beds, createdAt: Date.now() };
  db.wards.push(ward);
  res.status(201).json(ward);
});

app.put('/api/wards/:id', (req, res) => {
  const ward = db.wards.find((r) => r.id === Number(req.params.id));
  if (!ward) return res.status(404).json({ error: 'ward not found' });
  Object.assign(ward, req.body || {}, { id: ward.id });
  res.json(ward);
});

app.delete('/api/wards/:id', (req, res) => {
  const at = db.wards.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'ward not found' });
  db.wards.splice(at, 1);
  res.status(204).end();
});

// ---- supplies ----
app.get('/api/supplies', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.supplies;
  if (req.query.item) {
    const q = String(req.query.item).toLowerCase();
    rows = rows.filter((r) => String(r.item).toLowerCase().includes(q));
  }
  res.json({ supplies: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/supplies/:id', (req, res) => {
  const supply = db.supplies.find((r) => r.id === Number(req.params.id));
  if (!supply) return res.status(404).json({ error: 'supply not found' });
  res.json(supply);
});

app.post('/api/supplies', (req, res) => {
  const body = req.body || {};
  if (!body.item) return res.status(400).json({ error: 'item is required' });
  if (!body.qty) return res.status(400).json({ error: 'qty is required' });
  const supply = { id: nextId(db.supplies), item: body.item, qty: body.qty, createdAt: Date.now() };
  db.supplies.push(supply);
  res.status(201).json(supply);
});

app.put('/api/supplies/:id', (req, res) => {
  const supply = db.supplies.find((r) => r.id === Number(req.params.id));
  if (!supply) return res.status(404).json({ error: 'supply not found' });
  Object.assign(supply, req.body || {}, { id: supply.id });
  res.json(supply);
});

app.delete('/api/supplies/:id', (req, res) => {
  const at = db.supplies.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'supply not found' });
  db.supplies.splice(at, 1);
  res.status(204).end();
});

// ---- vaccines ----
app.get('/api/vaccines', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.vaccines;
  res.json({ vaccines: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/vaccines/:id', (req, res) => {
  const vaccine = db.vaccines.find((r) => r.id === Number(req.params.id));
  if (!vaccine) return res.status(404).json({ error: 'vaccine not found' });
  res.json(vaccine);
});

app.post('/api/vaccines', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.dose) return res.status(400).json({ error: 'dose is required' });
  const vaccine = { id: nextId(db.vaccines), name: body.name, dose: body.dose, createdAt: Date.now() };
  db.vaccines.push(vaccine);
  res.status(201).json(vaccine);
});

app.put('/api/vaccines/:id', (req, res) => {
  const vaccine = db.vaccines.find((r) => r.id === Number(req.params.id));
  if (!vaccine) return res.status(404).json({ error: 'vaccine not found' });
  Object.assign(vaccine, req.body || {}, { id: vaccine.id });
  res.json(vaccine);
});

app.delete('/api/vaccines/:id', (req, res) => {
  const at = db.vaccines.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'vaccine not found' });
  db.vaccines.splice(at, 1);
  res.status(204).end();
});

// ---- referrals ----
app.get('/api/referrals', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.referrals;
  res.json({ referrals: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/referrals/:id', (req, res) => {
  const referral = db.referrals.find((r) => r.id === Number(req.params.id));
  if (!referral) return res.status(404).json({ error: 'referral not found' });
  res.json(referral);
});

app.post('/api/referrals', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.clinic) return res.status(400).json({ error: 'clinic is required' });
  const referral = { id: nextId(db.referrals), patientId: body.patientId, clinic: body.clinic, createdAt: Date.now() };
  db.referrals.push(referral);
  res.status(201).json(referral);
});

app.put('/api/referrals/:id', (req, res) => {
  const referral = db.referrals.find((r) => r.id === Number(req.params.id));
  if (!referral) return res.status(404).json({ error: 'referral not found' });
  Object.assign(referral, req.body || {}, { id: referral.id });
  res.json(referral);
});

app.delete('/api/referrals/:id', (req, res) => {
  const at = db.referrals.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'referral not found' });
  db.referrals.splice(at, 1);
  res.status(204).end();
});

// ---- allergies ----
app.get('/api/allergies', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.allergies;
  res.json({ allergies: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/allergies/:id', (req, res) => {
  const allergy = db.allergies.find((r) => r.id === Number(req.params.id));
  if (!allergy) return res.status(404).json({ error: 'allergy not found' });
  res.json(allergy);
});

app.post('/api/allergies', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.substance) return res.status(400).json({ error: 'substance is required' });
  const allergy = { id: nextId(db.allergies), patientId: body.patientId, substance: body.substance, createdAt: Date.now() };
  db.allergies.push(allergy);
  res.status(201).json(allergy);
});

app.put('/api/allergies/:id', (req, res) => {
  const allergy = db.allergies.find((r) => r.id === Number(req.params.id));
  if (!allergy) return res.status(404).json({ error: 'allergy not found' });
  Object.assign(allergy, req.body || {}, { id: allergy.id });
  res.json(allergy);
});

app.delete('/api/allergies/:id', (req, res) => {
  const at = db.allergies.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'allergy not found' });
  db.allergies.splice(at, 1);
  res.status(204).end();
});

// ---- devices ----
app.get('/api/devices', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.devices;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ devices: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/devices/:id', (req, res) => {
  const device = db.devices.find((r) => r.id === Number(req.params.id));
  if (!device) return res.status(404).json({ error: 'device not found' });
  res.json(device);
});

app.post('/api/devices', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.room) return res.status(400).json({ error: 'room is required' });
  const device = { id: nextId(db.devices), name: body.name, room: body.room, createdAt: Date.now() };
  db.devices.push(device);
  res.status(201).json(device);
});

app.put('/api/devices/:id', (req, res) => {
  const device = db.devices.find((r) => r.id === Number(req.params.id));
  if (!device) return res.status(404).json({ error: 'device not found' });
  Object.assign(device, req.body || {}, { id: device.id });
  res.json(device);
});

app.delete('/api/devices/:id', (req, res) => {
  const at = db.devices.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'device not found' });
  db.devices.splice(at, 1);
  res.status(204).end();
});

// ---- shifts ----
app.get('/api/shifts', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.shifts;
  res.json({ shifts: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/shifts/:id', (req, res) => {
  const shift = db.shifts.find((r) => r.id === Number(req.params.id));
  if (!shift) return res.status(404).json({ error: 'shift not found' });
  res.json(shift);
});

app.post('/api/shifts', (req, res) => {
  const body = req.body || {};
  if (!body.staffId) return res.status(400).json({ error: 'staffId is required' });
  if (!body.day) return res.status(400).json({ error: 'day is required' });
  const shift = { id: nextId(db.shifts), staffId: body.staffId, day: body.day, createdAt: Date.now() };
  db.shifts.push(shift);
  res.status(201).json(shift);
});

app.put('/api/shifts/:id', (req, res) => {
  const shift = db.shifts.find((r) => r.id === Number(req.params.id));
  if (!shift) return res.status(404).json({ error: 'shift not found' });
  Object.assign(shift, req.body || {}, { id: shift.id });
  res.json(shift);
});

app.delete('/api/shifts/:id', (req, res) => {
  const at = db.shifts.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'shift not found' });
  db.shifts.splice(at, 1);
  res.status(204).end();
});

// ---- diagnoses ----
app.get('/api/diagnoses', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.diagnoses;
  res.json({ diagnoses: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/diagnoses/:id', (req, res) => {
  const diagnosis = db.diagnoses.find((r) => r.id === Number(req.params.id));
  if (!diagnosis) return res.status(404).json({ error: 'diagnosis not found' });
  res.json(diagnosis);
});

app.post('/api/diagnoses', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.code) return res.status(400).json({ error: 'code is required' });
  const diagnosis = { id: nextId(db.diagnoses), patientId: body.patientId, code: body.code, createdAt: Date.now() };
  db.diagnoses.push(diagnosis);
  res.status(201).json(diagnosis);
});

app.put('/api/diagnoses/:id', (req, res) => {
  const diagnosis = db.diagnoses.find((r) => r.id === Number(req.params.id));
  if (!diagnosis) return res.status(404).json({ error: 'diagnosis not found' });
  Object.assign(diagnosis, req.body || {}, { id: diagnosis.id });
  res.json(diagnosis);
});

app.delete('/api/diagnoses/:id', (req, res) => {
  const at = db.diagnoses.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'diagnosis not found' });
  db.diagnoses.splice(at, 1);
  res.status(204).end();
});

const REMINDER_HOURS_BEFORE = 24;

// Appointments that need a reminder now: they start in about a day and nobody was told yet.
function dueReminders(now = Date.now()) {
  const from = now + (REMINDER_HOURS_BEFORE - 1) * 3600 * 1000;
  const to = now + (REMINDER_HOURS_BEFORE + 1) * 3600 * 1000;
  return db.appointments.filter((a) => a.status === 'booked' && !a.reminded && a.start >= from && a.start < to);
}

// Run by the scheduler every hour: marks the appointments and returns the messages to send.
app.post('/api/reminders/run', (req, res) => {
  const due = dueReminders();
  const messages = due.map((a) => {
    a.reminded = true;
    const p = db.patients.find((x) => x.id === a.patientId);
    return { to: p ? p.phone : null, text: 'Your appointment is tomorrow at ' + new Date(a.start).toISOString().slice(11, 16) };
  });
  res.json({ sent: messages.length, messages });
});

// ---- therapies ----
app.get('/api/therapies', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.therapies;
  res.json({ therapies: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/therapies/:id', (req, res) => {
  const therapy = db.therapies.find((r) => r.id === Number(req.params.id));
  if (!therapy) return res.status(404).json({ error: 'therapy not found' });
  res.json(therapy);
});

app.post('/api/therapies', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.sessions) return res.status(400).json({ error: 'sessions is required' });
  const therapy = { id: nextId(db.therapies), name: body.name, sessions: body.sessions, createdAt: Date.now() };
  db.therapies.push(therapy);
  res.status(201).json(therapy);
});

app.put('/api/therapies/:id', (req, res) => {
  const therapy = db.therapies.find((r) => r.id === Number(req.params.id));
  if (!therapy) return res.status(404).json({ error: 'therapy not found' });
  Object.assign(therapy, req.body || {}, { id: therapy.id });
  res.json(therapy);
});

app.delete('/api/therapies/:id', (req, res) => {
  const at = db.therapies.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'therapy not found' });
  db.therapies.splice(at, 1);
  res.status(204).end();
});

// ---- pharmacies ----
app.get('/api/pharmacies', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.pharmacies;
  if (req.query.city) {
    const q = String(req.query.city).toLowerCase();
    rows = rows.filter((r) => String(r.city).toLowerCase().includes(q));
  }
  res.json({ pharmacies: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/pharmacies/:id', (req, res) => {
  const pharmacy = db.pharmacies.find((r) => r.id === Number(req.params.id));
  if (!pharmacy) return res.status(404).json({ error: 'pharmacy not found' });
  res.json(pharmacy);
});

app.post('/api/pharmacies', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.city) return res.status(400).json({ error: 'city is required' });
  if (!body.phone) return res.status(400).json({ error: 'phone is required' });
  const pharmacy = { id: nextId(db.pharmacies), name: body.name, city: body.city, phone: body.phone, createdAt: Date.now() };
  db.pharmacies.push(pharmacy);
  res.status(201).json(pharmacy);
});

app.put('/api/pharmacies/:id', (req, res) => {
  const pharmacy = db.pharmacies.find((r) => r.id === Number(req.params.id));
  if (!pharmacy) return res.status(404).json({ error: 'pharmacy not found' });
  Object.assign(pharmacy, req.body || {}, { id: pharmacy.id });
  res.json(pharmacy);
});

app.delete('/api/pharmacies/:id', (req, res) => {
  const at = db.pharmacies.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'pharmacy not found' });
  db.pharmacies.splice(at, 1);
  res.status(204).end();
});

// ---- transports ----
app.get('/api/transports', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.transports;
  res.json({ transports: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/transports/:id', (req, res) => {
  const transport = db.transports.find((r) => r.id === Number(req.params.id));
  if (!transport) return res.status(404).json({ error: 'transport not found' });
  res.json(transport);
});

app.post('/api/transports', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.pickup) return res.status(400).json({ error: 'pickup is required' });
  const transport = { id: nextId(db.transports), patientId: body.patientId, pickup: body.pickup, createdAt: Date.now() };
  db.transports.push(transport);
  res.status(201).json(transport);
});

app.put('/api/transports/:id', (req, res) => {
  const transport = db.transports.find((r) => r.id === Number(req.params.id));
  if (!transport) return res.status(404).json({ error: 'transport not found' });
  Object.assign(transport, req.body || {}, { id: transport.id });
  res.json(transport);
});

app.delete('/api/transports/:id', (req, res) => {
  const at = db.transports.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'transport not found' });
  db.transports.splice(at, 1);
  res.status(204).end();
});

// ---- donors ----
app.get('/api/donors', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.donors;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ donors: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/donors/:id', (req, res) => {
  const donor = db.donors.find((r) => r.id === Number(req.params.id));
  if (!donor) return res.status(404).json({ error: 'donor not found' });
  res.json(donor);
});

app.post('/api/donors', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.blood) return res.status(400).json({ error: 'blood is required' });
  const donor = { id: nextId(db.donors), name: body.name, blood: body.blood, createdAt: Date.now() };
  db.donors.push(donor);
  res.status(201).json(donor);
});

app.put('/api/donors/:id', (req, res) => {
  const donor = db.donors.find((r) => r.id === Number(req.params.id));
  if (!donor) return res.status(404).json({ error: 'donor not found' });
  Object.assign(donor, req.body || {}, { id: donor.id });
  res.json(donor);
});

app.delete('/api/donors/:id', (req, res) => {
  const at = db.donors.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'donor not found' });
  db.donors.splice(at, 1);
  res.status(204).end();
});

// ---- archives ----
app.get('/api/archives', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.archives;
  res.json({ archives: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/archives/:id', (req, res) => {
  const archive = db.archives.find((r) => r.id === Number(req.params.id));
  if (!archive) return res.status(404).json({ error: 'archive not found' });
  res.json(archive);
});

app.post('/api/archives', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.box) return res.status(400).json({ error: 'box is required' });
  const archive = { id: nextId(db.archives), patientId: body.patientId, box: body.box, createdAt: Date.now() };
  db.archives.push(archive);
  res.status(201).json(archive);
});

app.put('/api/archives/:id', (req, res) => {
  const archive = db.archives.find((r) => r.id === Number(req.params.id));
  if (!archive) return res.status(404).json({ error: 'archive not found' });
  Object.assign(archive, req.body || {}, { id: archive.id });
  res.json(archive);
});

app.delete('/api/archives/:id', (req, res) => {
  const at = db.archives.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'archive not found' });
  db.archives.splice(at, 1);
  res.status(204).end();
});

// ---- tariffs ----
app.get('/api/tariffs', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.tariffs;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ tariffs: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/tariffs/:id', (req, res) => {
  const tariff = db.tariffs.find((r) => r.id === Number(req.params.id));
  if (!tariff) return res.status(404).json({ error: 'tariff not found' });
  res.json(tariff);
});

app.post('/api/tariffs', (req, res) => {
  const body = req.body || {};
  if (!body.code) return res.status(400).json({ error: 'code is required' });
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.price) return res.status(400).json({ error: 'price is required' });
  const tariff = { id: nextId(db.tariffs), code: body.code, name: body.name, price: body.price, createdAt: Date.now() };
  db.tariffs.push(tariff);
  res.status(201).json(tariff);
});

app.put('/api/tariffs/:id', (req, res) => {
  const tariff = db.tariffs.find((r) => r.id === Number(req.params.id));
  if (!tariff) return res.status(404).json({ error: 'tariff not found' });
  Object.assign(tariff, req.body || {}, { id: tariff.id });
  res.json(tariff);
});

app.delete('/api/tariffs/:id', (req, res) => {
  const at = db.tariffs.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'tariff not found' });
  db.tariffs.splice(at, 1);
  res.status(204).end();
});

// ---- waitlist ----
app.get('/api/waitlist', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.waitlist;
  res.json({ waitlist: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/waitlist/:id', (req, res) => {
  const entry = db.waitlist.find((r) => r.id === Number(req.params.id));
  if (!entry) return res.status(404).json({ error: 'entry not found' });
  res.json(entry);
});

app.post('/api/waitlist', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.doctorId) return res.status(400).json({ error: 'doctorId is required' });
  const entry = { id: nextId(db.waitlist), patientId: body.patientId, doctorId: body.doctorId, createdAt: Date.now() };
  db.waitlist.push(entry);
  res.status(201).json(entry);
});

app.put('/api/waitlist/:id', (req, res) => {
  const entry = db.waitlist.find((r) => r.id === Number(req.params.id));
  if (!entry) return res.status(404).json({ error: 'entry not found' });
  Object.assign(entry, req.body || {}, { id: entry.id });
  res.json(entry);
});

app.delete('/api/waitlist/:id', (req, res) => {
  const at = db.waitlist.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'entry not found' });
  db.waitlist.splice(at, 1);
  res.status(204).end();
});

// ---- complaints ----
app.get('/api/complaints', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.complaints;
  res.json({ complaints: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/complaints/:id', (req, res) => {
  const complaint = db.complaints.find((r) => r.id === Number(req.params.id));
  if (!complaint) return res.status(404).json({ error: 'complaint not found' });
  res.json(complaint);
});

app.post('/api/complaints', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.text) return res.status(400).json({ error: 'text is required' });
  const complaint = { id: nextId(db.complaints), patientId: body.patientId, text: body.text, createdAt: Date.now() };
  db.complaints.push(complaint);
  res.status(201).json(complaint);
});

app.put('/api/complaints/:id', (req, res) => {
  const complaint = db.complaints.find((r) => r.id === Number(req.params.id));
  if (!complaint) return res.status(404).json({ error: 'complaint not found' });
  Object.assign(complaint, req.body || {}, { id: complaint.id });
  res.json(complaint);
});

app.delete('/api/complaints/:id', (req, res) => {
  const at = db.complaints.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'complaint not found' });
  db.complaints.splice(at, 1);
  res.status(204).end();
});

// ---- rotas ----
app.get('/api/rotas', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.rotas;
  res.json({ rotas: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/rotas/:id', (req, res) => {
  const rota = db.rotas.find((r) => r.id === Number(req.params.id));
  if (!rota) return res.status(404).json({ error: 'rota not found' });
  res.json(rota);
});

app.post('/api/rotas', (req, res) => {
  const body = req.body || {};
  if (!body.week) return res.status(400).json({ error: 'week is required' });
  if (!body.ward) return res.status(400).json({ error: 'ward is required' });
  const rota = { id: nextId(db.rotas), week: body.week, ward: body.ward, createdAt: Date.now() };
  db.rotas.push(rota);
  res.status(201).json(rota);
});

app.put('/api/rotas/:id', (req, res) => {
  const rota = db.rotas.find((r) => r.id === Number(req.params.id));
  if (!rota) return res.status(404).json({ error: 'rota not found' });
  Object.assign(rota, req.body || {}, { id: rota.id });
  res.json(rota);
});

app.delete('/api/rotas/:id', (req, res) => {
  const at = db.rotas.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'rota not found' });
  db.rotas.splice(at, 1);
  res.status(204).end();
});

// ---- pharmacists ----
app.get('/api/pharmacists', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.pharmacists;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ pharmacists: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/pharmacists/:id', (req, res) => {
  const pharmacist = db.pharmacists.find((r) => r.id === Number(req.params.id));
  if (!pharmacist) return res.status(404).json({ error: 'pharmacist not found' });
  res.json(pharmacist);
});

app.post('/api/pharmacists', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.license) return res.status(400).json({ error: 'license is required' });
  const pharmacist = { id: nextId(db.pharmacists), name: body.name, license: body.license, createdAt: Date.now() };
  db.pharmacists.push(pharmacist);
  res.status(201).json(pharmacist);
});

app.put('/api/pharmacists/:id', (req, res) => {
  const pharmacist = db.pharmacists.find((r) => r.id === Number(req.params.id));
  if (!pharmacist) return res.status(404).json({ error: 'pharmacist not found' });
  Object.assign(pharmacist, req.body || {}, { id: pharmacist.id });
  res.json(pharmacist);
});

app.delete('/api/pharmacists/:id', (req, res) => {
  const at = db.pharmacists.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'pharmacist not found' });
  db.pharmacists.splice(at, 1);
  res.status(204).end();
});

// ---- staff ----
app.get('/api/staff', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.staff;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ staff: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/staff/:id', (req, res) => {
  const member = db.staff.find((r) => r.id === Number(req.params.id));
  if (!member) return res.status(404).json({ error: 'member not found' });
  res.json(member);
});

app.post('/api/staff', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.role) return res.status(400).json({ error: 'role is required' });
  if (!body.phone) return res.status(400).json({ error: 'phone is required' });
  const member = { id: nextId(db.staff), name: body.name, role: body.role, phone: body.phone, createdAt: Date.now() };
  db.staff.push(member);
  res.status(201).json(member);
});

app.put('/api/staff/:id', (req, res) => {
  const member = db.staff.find((r) => r.id === Number(req.params.id));
  if (!member) return res.status(404).json({ error: 'member not found' });
  Object.assign(member, req.body || {}, { id: member.id });
  res.json(member);
});

app.delete('/api/staff/:id', (req, res) => {
  const at = db.staff.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'member not found' });
  db.staff.splice(at, 1);
  res.status(204).end();
});

// ---- clinics ----
app.get('/api/clinics', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.clinics;
  if (req.query.city) {
    const q = String(req.query.city).toLowerCase();
    rows = rows.filter((r) => String(r.city).toLowerCase().includes(q));
  }
  res.json({ clinics: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/clinics/:id', (req, res) => {
  const clinic = db.clinics.find((r) => r.id === Number(req.params.id));
  if (!clinic) return res.status(404).json({ error: 'clinic not found' });
  res.json(clinic);
});

app.post('/api/clinics', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.city) return res.status(400).json({ error: 'city is required' });
  const clinic = { id: nextId(db.clinics), name: body.name, city: body.city, createdAt: Date.now() };
  db.clinics.push(clinic);
  res.status(201).json(clinic);
});

app.put('/api/clinics/:id', (req, res) => {
  const clinic = db.clinics.find((r) => r.id === Number(req.params.id));
  if (!clinic) return res.status(404).json({ error: 'clinic not found' });
  Object.assign(clinic, req.body || {}, { id: clinic.id });
  res.json(clinic);
});

app.delete('/api/clinics/:id', (req, res) => {
  const at = db.clinics.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'clinic not found' });
  db.clinics.splice(at, 1);
  res.status(204).end();
});

// ---- forms ----
app.get('/api/forms', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.forms;
  res.json({ forms: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/forms/:id', (req, res) => {
  const form = db.forms.find((r) => r.id === Number(req.params.id));
  if (!form) return res.status(404).json({ error: 'form not found' });
  res.json(form);
});

app.post('/api/forms', (req, res) => {
  const body = req.body || {};
  if (!body.title) return res.status(400).json({ error: 'title is required' });
  if (!body.version) return res.status(400).json({ error: 'version is required' });
  const form = { id: nextId(db.forms), title: body.title, version: body.version, createdAt: Date.now() };
  db.forms.push(form);
  res.status(201).json(form);
});

app.put('/api/forms/:id', (req, res) => {
  const form = db.forms.find((r) => r.id === Number(req.params.id));
  if (!form) return res.status(404).json({ error: 'form not found' });
  Object.assign(form, req.body || {}, { id: form.id });
  res.json(form);
});

app.delete('/api/forms/:id', (req, res) => {
  const at = db.forms.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'form not found' });
  db.forms.splice(at, 1);
  res.status(204).end();
});

app.get('/api/invoices/:id/total', (req, res) => {
  const invoice = db.invoices.find((i) => i.id === Number(req.params.id));
  if (!invoice) return res.status(404).json({ error: 'invoice not found' });
  res.json({ id: invoice.id, total: billing.invoiceTotal(invoice, db.tariffs) });
});

app.get('/api/reports/monthly', (req, res) => {
  res.json(reports.monthlyVisits(db.visits, db.appointments));
});

function freeSlot(doctorId, start) {
  taken.delete(slotKey(doctorId, start));
}

// Cancelling an appointment gives its slot back, so the next patient can book the same time.
app.post('/api/appointments/:id/cancel', (req, res) => {
  const appt = db.appointments.find((a) => a.id === Number(req.params.id));
  if (!appt) return res.status(404).json({ error: 'appointment not found' });
  if (appt.status === 'cancelled') return res.status(409).json({ error: 'already cancelled' });
  appt.status = 'cancelled';
  freeSlot(appt.doctorId, appt.start);
  res.json({ id: appt.id, status: appt.status, slotFree: true });
});

// ---- payments ----
app.get('/api/payments', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.payments;
  res.json({ payments: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/payments/:id', (req, res) => {
  const payment = db.payments.find((r) => r.id === Number(req.params.id));
  if (!payment) return res.status(404).json({ error: 'payment not found' });
  res.json(payment);
});

app.post('/api/payments', (req, res) => {
  const body = req.body || {};
  if (!body.invoiceId) return res.status(400).json({ error: 'invoiceId is required' });
  if (!body.amount) return res.status(400).json({ error: 'amount is required' });
  if (!body.method) return res.status(400).json({ error: 'method is required' });
  const payment = { id: nextId(db.payments), invoiceId: body.invoiceId, amount: body.amount, method: body.method, createdAt: Date.now() };
  db.payments.push(payment);
  res.status(201).json(payment);
});

app.put('/api/payments/:id', (req, res) => {
  const payment = db.payments.find((r) => r.id === Number(req.params.id));
  if (!payment) return res.status(404).json({ error: 'payment not found' });
  Object.assign(payment, req.body || {}, { id: payment.id });
  res.json(payment);
});

app.delete('/api/payments/:id', (req, res) => {
  const at = db.payments.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'payment not found' });
  db.payments.splice(at, 1);
  res.status(204).end();
});

// ---- refunds ----
app.get('/api/refunds', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.refunds;
  res.json({ refunds: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/refunds/:id', (req, res) => {
  const refund = db.refunds.find((r) => r.id === Number(req.params.id));
  if (!refund) return res.status(404).json({ error: 'refund not found' });
  res.json(refund);
});

app.post('/api/refunds', (req, res) => {
  const body = req.body || {};
  if (!body.paymentId) return res.status(400).json({ error: 'paymentId is required' });
  if (!body.reason) return res.status(400).json({ error: 'reason is required' });
  const refund = { id: nextId(db.refunds), paymentId: body.paymentId, reason: body.reason, createdAt: Date.now() };
  db.refunds.push(refund);
  res.status(201).json(refund);
});

app.put('/api/refunds/:id', (req, res) => {
  const refund = db.refunds.find((r) => r.id === Number(req.params.id));
  if (!refund) return res.status(404).json({ error: 'refund not found' });
  Object.assign(refund, req.body || {}, { id: refund.id });
  res.json(refund);
});

app.delete('/api/refunds/:id', (req, res) => {
  const at = db.refunds.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'refund not found' });
  db.refunds.splice(at, 1);
  res.status(204).end();
});

// ---- schedules ----
app.get('/api/schedules', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.schedules;
  res.json({ schedules: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/schedules/:id', (req, res) => {
  const schedule = db.schedules.find((r) => r.id === Number(req.params.id));
  if (!schedule) return res.status(404).json({ error: 'schedule not found' });
  res.json(schedule);
});

app.post('/api/schedules', (req, res) => {
  const body = req.body || {};
  if (!body.doctorId) return res.status(400).json({ error: 'doctorId is required' });
  if (!body.weekday) return res.status(400).json({ error: 'weekday is required' });
  const schedule = { id: nextId(db.schedules), doctorId: body.doctorId, weekday: body.weekday, createdAt: Date.now() };
  db.schedules.push(schedule);
  res.status(201).json(schedule);
});

app.put('/api/schedules/:id', (req, res) => {
  const schedule = db.schedules.find((r) => r.id === Number(req.params.id));
  if (!schedule) return res.status(404).json({ error: 'schedule not found' });
  Object.assign(schedule, req.body || {}, { id: schedule.id });
  res.json(schedule);
});

app.delete('/api/schedules/:id', (req, res) => {
  const at = db.schedules.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'schedule not found' });
  db.schedules.splice(at, 1);
  res.status(204).end();
});

// ---- equipment ----
app.get('/api/equipment', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.equipment;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ equipment: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/equipment/:id', (req, res) => {
  const item = db.equipment.find((r) => r.id === Number(req.params.id));
  if (!item) return res.status(404).json({ error: 'item not found' });
  res.json(item);
});

app.post('/api/equipment', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.room) return res.status(400).json({ error: 'room is required' });
  const item = { id: nextId(db.equipment), name: body.name, room: body.room, createdAt: Date.now() };
  db.equipment.push(item);
  res.status(201).json(item);
});

app.put('/api/equipment/:id', (req, res) => {
  const item = db.equipment.find((r) => r.id === Number(req.params.id));
  if (!item) return res.status(404).json({ error: 'item not found' });
  Object.assign(item, req.body || {}, { id: item.id });
  res.json(item);
});

app.delete('/api/equipment/:id', (req, res) => {
  const at = db.equipment.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'item not found' });
  db.equipment.splice(at, 1);
  res.status(204).end();
});

// ---- contacts ----
app.get('/api/contacts', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.contacts;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ contacts: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/contacts/:id', (req, res) => {
  const contact = db.contacts.find((r) => r.id === Number(req.params.id));
  if (!contact) return res.status(404).json({ error: 'contact not found' });
  res.json(contact);
});

app.post('/api/contacts', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.phone) return res.status(400).json({ error: 'phone is required' });
  const contact = { id: nextId(db.contacts), patientId: body.patientId, name: body.name, phone: body.phone, createdAt: Date.now() };
  db.contacts.push(contact);
  res.status(201).json(contact);
});

app.put('/api/contacts/:id', (req, res) => {
  const contact = db.contacts.find((r) => r.id === Number(req.params.id));
  if (!contact) return res.status(404).json({ error: 'contact not found' });
  Object.assign(contact, req.body || {}, { id: contact.id });
  res.json(contact);
});

app.delete('/api/contacts/:id', (req, res) => {
  const at = db.contacts.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'contact not found' });
  db.contacts.splice(at, 1);
  res.status(204).end();
});

// ---- alerts ----
app.get('/api/alerts', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.alerts;
  res.json({ alerts: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/alerts/:id', (req, res) => {
  const alert = db.alerts.find((r) => r.id === Number(req.params.id));
  if (!alert) return res.status(404).json({ error: 'alert not found' });
  res.json(alert);
});

app.post('/api/alerts', (req, res) => {
  const body = req.body || {};
  if (!body.patientId) return res.status(400).json({ error: 'patientId is required' });
  if (!body.text) return res.status(400).json({ error: 'text is required' });
  const alert = { id: nextId(db.alerts), patientId: body.patientId, text: body.text, createdAt: Date.now() };
  db.alerts.push(alert);
  res.status(201).json(alert);
});

app.put('/api/alerts/:id', (req, res) => {
  const alert = db.alerts.find((r) => r.id === Number(req.params.id));
  if (!alert) return res.status(404).json({ error: 'alert not found' });
  Object.assign(alert, req.body || {}, { id: alert.id });
  res.json(alert);
});

app.delete('/api/alerts/:id', (req, res) => {
  const at = db.alerts.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'alert not found' });
  db.alerts.splice(at, 1);
  res.status(204).end();
});

app.use((req, res) => {
  res.status(404).json({ error: 'not found' });
});

app.listen(PORT, () => {
  console.log('clinic listening on ' + PORT);
});
