// audit2.mjs — classify uncovered literals as user-visible vs log-only
import fs from 'fs';
import path from 'path';

const root = path.resolve(import.meta.dirname, '..');
const base = path.join(root, 'backend', 'src');
const locale = JSON.parse(fs.readFileSync(path.join(root, 'backend', 'locales', 'en_US.json'), 'utf8'));
const exact = new Set(Object.keys(locale.exact || {}));
const prefixes = (locale.prefix || []).map(([p]) => p);

const CS_RE = /[\u4e00-\u9fa5]/;
const STR_RE = /"([^"\\]*(?:\\.[^"\\]*)*)"/g;
const RESP_RE = /Resp::|"msg"|msg\s*[:=]\s*"/;
const LOG_RE = /log::|warn!|error!|info!|debug!|println!/;

const files = [];
function walk(d) {
  for (const ent of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, ent.name);
    if (ent.isDirectory()) walk(p);
    else if (ent.name.endsWith('.rs')) files.push(p);
  }
}
walk(base);

const uncovered = [];
for (const f of files) {
  const rel = path.relative(root, f).replace(/\\/g, '/');
  const lines = fs.readFileSync(f, 'utf8').split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    let m;
    while ((m = STR_RE.exec(line)) !== null) {
      const s = m[1];
      if (!CS_RE.test(s)) continue;
      if (exact.has(s)) continue;
      let hit = false;
      for (const p of prefixes) { if (s.startsWith(p)) { hit = true; break; } }
      if (hit) continue;
      const isResp = RESP_RE.test(line);
      const isLog = LOG_RE.test(line);
      uncovered.push({ file: rel, line: i + 1, lit: s, isResp, isLog });
    }
  }
}

const byLit = new Map();
for (const u of uncovered) {
  if (!byLit.has(u.lit)) byLit.set(u.lit, []);
  byLit.get(u.lit).push(u);
}

const arr = [...byLit.entries()].map(([lit, a]) => ({
  lit, count: a.length,
  isResp: a.some(x => x.isResp),
  isLog: a.some(x => x.isLog),
  sample: a[0].file + ':' + a[0].line
})).sort((a, b) => (b.isResp - a.isResp) || (a.isLog - b.isLog) || b.count - a.count);

console.log('UNCOVERED unique:', arr.length, 'resp:', arr.filter(a => a.isResp).length, 'log:', arr.filter(a => a.isLog && !a.isResp).length);
console.log();
for (const it of arr) {
  const tag = it.isResp ? '[RESP]' : (it.isLog ? '[LOG ]' : '[othr]');
  console.log(tag, JSON.stringify(it.lit), '(' + it.count + 'x)', it.sample);
}
