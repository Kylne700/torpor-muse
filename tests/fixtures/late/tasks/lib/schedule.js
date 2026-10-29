'use strict';
// Date helpers and the rules of the task list: when a task is due, when it comes back, how tasks are ordered.

const MINUTE_MS = 60 * 1000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

const PRIORITY_RANK = { urgent: 0, high: 1, normal: 2, low: 3 };
const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTH_NAMES = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

function startOfDay(date) {
  const d = new Date(date);
  d.setHours(0, 0, 0, 0);
  return d;
}

function addDays(date, n) {
  const d = new Date(date);
  d.setDate(d.getDate() + n);
  return d;
}

function isSameDay(a, b) {
  return startOfDay(a).getTime() === startOfDay(b).getTime();
}

function daysBetween(a, b) {
  return Math.round((startOfDay(b) - startOfDay(a)) / DAY_MS);
}

// Accepts 2025-03-09 or 2025-03-09 14:30 and returns a Date, or null when the text is not a date.
function parseDue(text) {
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2}))?$/.exec(String(text).trim());
  if (!m) return null;
  const [, y, mo, d, h = '0', mi = '0'] = m;
  const date = new Date(Number(y), Number(mo) - 1, Number(d), Number(h), Number(mi));
  return Number.isNaN(date.getTime()) ? null : date;
}

function formatDue(date) {
  const d = new Date(date);
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  return DAY_NAMES[d.getDay()] + ' ' + d.getDate() + ' ' + MONTH_NAMES[d.getMonth()] + ', ' + hh + ':' + mm;
}

class TaskList {
  constructor(items = []) {
    this.items = items;
    this.nextId = items.reduce((max, t) => Math.max(max, t.id || 0), 0) + 1;
  }

  add(fields) {
    const task = { id: this.nextId++, done: false, priority: 'normal', tags: [], ...fields };
    this.items.push(task);
    return task;
  }

  find(id) {
    return this.items.find((t) => t.id === id) || null;
  }

  update(id, fields) {
    const task = this.find(id);
    if (!task) return null;
    Object.assign(task, fields, { id });
    return task;
  }

  remove(id) {
    const at = this.items.findIndex((t) => t.id === id);
    if (at < 0) return false;
    this.items.splice(at, 1);
    return true;
  }

  toggle(id) {
    const task = this.find(id);
    if (!task) return null;
    task.done = !task.done;
    task.doneAt = task.done ? Date.now() : null;
    return task;
  }
}

// Reads 1h30m, 45m or 2h into minutes; plain numbers are minutes too.
function parseDuration(text) {
  const t = String(text).trim().toLowerCase();
  if (/^\d+$/.test(t)) return Number(t);
  const m = /^(?:(\d+)h)?\s*(?:(\d+)m)?$/.exec(t);
  if (!m || (!m[1] && !m[2])) return null;
  return Number(m[1] || 0) * 60 + Number(m[2] || 0);
}

function badgeColor(task) {
  if (task.done) return 'grey';
  if (task.priority === 'urgent') return 'red';
  if (task.priority === 'high') return 'orange';
  return task.priority === 'low' ? 'blue' : 'green';
}

function describe(task) {
  const parts = [task.title];
  if (task.due) parts.push('due ' + formatDue(task.due));
  if (task.estimate) parts.push(formatMinutes(task.estimate));
  if (task.tags && task.tags.length) parts.push('#' + task.tags.join(' #'));
  return parts.join(' - ');
}

function formatMinutes(total) {
  const h = Math.floor(total / 60);
  const m = total % 60;
  if (h === 0) return m + ' min';
  return m === 0 ? h + ' h' : h + ' h ' + m + ' min';
}

function parseTags(text) {
  return String(text || '')
    .split(/[ ,]+/)
    .map((t) => t.replace(/^#/, '').toLowerCase())
    .filter(Boolean);
}

function groupByDay(tasks) {
  const groups = new Map();
  for (const t of tasks) {
    if (!t.due) continue;
    const key = startOfDay(t.due).toISOString().slice(0, 10);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(t);
  }
  return groups;
}

// A task is overdue when it has a due time in the past and is not done.
function isOverdue(task, now = new Date()) {
  if (task.done || !task.due) return false;
  return new Date(task.due).getTime() < now.getTime();
}

// The tasks that get the red badge on the list page.
function overdueTasks(tasks, now = new Date()) {
  return tasks.filter((t) => isOverdue(t, now));
}

function weekNumber(date) {
  const d = new Date(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()));
  d.setUTCDate(d.getUTCDate() + 4 - (d.getUTCDay() || 7));
  const yearStart = new Date(Date.UTC(d.getUTCFullYear(), 0, 1));
  return Math.ceil(((d - yearStart) / DAY_MS + 1) / 7);
}

function calendarGrid(year, month) {
  const first = new Date(year, month, 1);
  const days = new Date(year, month + 1, 0).getDate();
  const cells = [];
  for (let i = 0; i < first.getDay(); i++) cells.push(null);
  for (let d = 1; d <= days; d++) cells.push(new Date(year, month, d));
  while (cells.length % 7 !== 0) cells.push(null);
  const rows = [];
  for (let i = 0; i < cells.length; i += 7) rows.push(cells.slice(i, i + 7));
  return rows;
}

function workingDaysBetween(a, b) {
  let count = 0;
  for (let d = startOfDay(a); d < startOfDay(b); d = addDays(d, 1)) {
    if (d.getDay() !== 5 && d.getDay() !== 6) count++;
  }
  return count;
}

function summarize(tasks) {
  const open = tasks.filter((t) => !t.done);
  const minutes = open.reduce((sum, t) => sum + (t.estimate || 0), 0);
  return { open: open.length, done: tasks.length - open.length, minutes, text: formatMinutes(minutes) };
}

function busiestDay(tasks) {
  let best = null;
  for (const [day, list] of groupByDay(tasks)) {
    if (!best || list.length > best.count) best = { day, count: list.length };
  }
  return best;
}

function tasksForWeek(tasks, from = new Date()) {
  const start = startOfDay(from);
  const end = addDays(start, 7);
  return tasks.filter((t) => t.due && new Date(t.due) >= start && new Date(t.due) < end);
}

function freeMinutes(tasks, day, workday = 8 * 60) {
  const booked = tasks
    .filter((t) => t.due && isSameDay(t.due, day) && !t.done)
    .reduce((sum, t) => sum + (t.estimate || 30), 0);
  return Math.max(0, workday - booked);
}

// Search box syntax: free words plus tag:name and prio:high.
function filterTasks(tasks, query) {
  const words = [];
  let tag = null;
  let prio = null;
  for (const part of String(query || '').split(/\s+/).filter(Boolean)) {
    if (part.startsWith('tag:')) tag = part.slice(4).toLowerCase();
    else if (part.startsWith('prio:')) prio = part.slice(5).toLowerCase();
    else words.push(part.toLowerCase());
  }
  return tasks.filter((t) => {
    if (tag && !(t.tags || []).includes(tag)) return false;
    if (prio && t.priority !== prio) return false;
    return words.every((w) => String(t.title).toLowerCase().includes(w));
  });
}

function mergeDuplicates(tasks) {
  const seen = new Map();
  for (const t of tasks) {
    const key = String(t.title).trim().toLowerCase() + '|' + (t.due || '');
    if (!seen.has(key)) seen.set(key, t);
    else seen.get(key).tags = [...new Set([...(seen.get(key).tags || []), ...(t.tags || [])])];
  }
  return [...seen.values()];
}

function statsByTag(tasks) {
  const out = {};
  for (const t of tasks) {
    for (const tag of t.tags || []) {
      out[tag] = out[tag] || { open: 0, done: 0 };
      out[tag][t.done ? 'done' : 'open']++;
    }
  }
  return out;
}

// The next time a repeating task is due. `repeat` is 'daily', 'weekly' or 'monthly'; a weekly task keeps its weekday.
function nextOccurrence(task) {
  const due = new Date(task.due);
  switch (task.repeat) {
    case 'daily':
      return addDays(due, 1);
    case 'weekly':
      return addDays(due, 7);
    case 'monthly': {
      const d = new Date(due);
      const day = d.getDate();
      d.setDate(1);
      d.setMonth(d.getMonth() + 1);
      const last = new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate();
      d.setDate(Math.min(day, last));
      return d;
    }
    default:
      return null;
  }
}

// When a repeating task is finished, a fresh copy is made for its next time.
function completeTask(task) {
  const done = { ...task, done: true, doneAt: Date.now() };
  const next = task.repeat ? nextOccurrence(task) : null;
  if (!next) return { done, next: null };
  return { done, next: { ...task, id: undefined, due: next.toISOString(), done: false } };
}

function importLines(text) {
  const out = [];
  for (const line of String(text).split('\n')) {
    const [title, due, priority] = line.split(';').map((s) => s.trim());
    if (!title) continue;
    out.push({ title, due: due ? (parseDue(due) || new Date(due)).toISOString() : null, priority: priority || 'normal' });
  }
  return out;
}

function exportLines(tasks) {
  return tasks.map((t) => [t.title, t.due ? formatDue(t.due) : '', t.priority || 'normal'].join(';')).join('\n');
}

function longestOpen(tasks) {
  return tasks.filter((t) => !t.done).sort((a, b) => (b.estimate || 0) - (a.estimate || 0))[0] || null;
}

function dueSoon(tasks, withinHours = 24, now = new Date()) {
  const limit = now.getTime() + withinHours * HOUR_MS;
  return tasks.filter((t) => !t.done && t.due && new Date(t.due).getTime() <= limit);
}

function archiveOld(tasks, days = 30, now = new Date()) {
  const limit = now.getTime() - days * DAY_MS;
  const keep = [];
  const old = [];
  for (const t of tasks) (t.done && t.doneAt && t.doneAt < limit ? old : keep).push(t);
  return { keep, old };
}

// Order for the list page: priority first, then the earliest due time; tasks with no due time go last.
function compareTasks(a, b) {
  const byPriority = (PRIORITY_RANK[a.priority] ?? 2) - (PRIORITY_RANK[b.priority] ?? 2);
  if (byPriority !== 0) return byPriority;
  const ad = a.due ? new Date(a.due).getTime() : Infinity;
  const bd = b.due ? new Date(b.due).getTime() : Infinity;
  return ad === bd ? 0 : ad < bd ? -1 : 1;
}

function sortTasks(tasks) {
  return [...tasks].sort(compareTasks);
}

// Moves a task to a later time. With no `minutes` it moves by an hour.
function postpone(task, minutes = 60) {
  const base = task.due ? new Date(task.due) : new Date();
  base.setMinutes(base.getMinutes() + minutes);
  return { ...task, due: base.toISOString(), postponed: (task.postponed || 0) + 1 };
}

module.exports = {
  startOfDay, addDays, isSameDay, daysBetween, parseDue, formatDue, isOverdue, overdueTasks, describe,
  formatMinutes, parseTags, groupByDay, summarize, busiestDay, tasksForWeek, freeMinutes, nextOccurrence,
  completeTask, importLines, exportLines, longestOpen, dueSoon, compareTasks, sortTasks, postpone,
};
