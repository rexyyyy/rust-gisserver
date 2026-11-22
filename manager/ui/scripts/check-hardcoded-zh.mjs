// Report Chinese text that is still hardcoded outside the message catalogs.
// Comments are stripped first: short English comments are the project convention.
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';

const root = resolve(process.cwd(), 'src');
const excluded = new Set([join(root, 'lang')]);
const hasHan = (line) => /[\u3400-\u9fff]/.test(line);

function stripComments(text, ext) {
  const isVue = ext === '.vue';
  let inBlock = false;
  return text
    .split('\n')
    .map((line) => {
      let out = '';
      let i = 0;
      while (i < line.length) {
        if (inBlock) {
          const end = line.indexOf('*/', i);
          const htmlEnd = isVue ? line.indexOf('-->', i) : -1;
          const stop = [end, htmlEnd].filter((n) => n >= 0).sort((a, b) => a - b)[0];
          if (stop === undefined) return out;
          i = stop + (stop === end ? 2 : 3);
          inBlock = false;
          continue;
        }
        const block = line.indexOf('/*', i);
        const html = isVue ? line.indexOf('<!--', i) : -1;
        const lineComment = line.indexOf('//', i);
        const starts = [block, html, lineComment].filter((n) => n >= 0).sort((a, b) => a - b);
        if (starts.length === 0) {
          out += line.slice(i);
          break;
        }
        const start = starts[0];
        out += line.slice(i, start);
        if (start === block || start === html) {
          inBlock = true;
          i = start + (start === block ? 2 : 4);
        } else {
          break;
        }
      }
      return out;
    })
    .join('\n');
}

function walk(dir) {
  return readdirSync(dir).flatMap((entry) => {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      return [...excluded].some((skip) => full === skip || full.startsWith(skip + '\\') || full.startsWith(skip + '/'))
        ? []
        : walk(full);
    }
    return /\.(vue|ts|tsx)$/.test(full) ? [full] : [];
  });
}

const report = [];
for (const file of walk(root)) {
  const text = readFileSync(file, 'utf8');
  const ext = file.slice(file.lastIndexOf('.'));
  const clean = stripComments(text, ext);
  const lines = clean.split('\n').map((line, idx) => ({ line, no: idx + 1 })).filter((row) => hasHan(row.line));
  if (lines.length) {
    report.push({ file: relative(root, file), count: lines.length, samples: lines.slice(0, 3) });
  }
}

report.sort((a, b) => b.count - a.count);
const total = report.reduce((sum, item) => sum + item.count, 0);
console.log(`Hardcoded Chinese lines outside src/lang: ${total} in ${report.length} files`);
for (const item of report.slice(0, 40)) {
  console.log(`  ${String(item.count).padStart(4)}  ${item.file}`);
  for (const sample of item.samples) console.log(`         L${sample.no}: ${sample.line.trim().slice(0, 100)}`);
}
if (report.length > 40) console.log(`  ... ${report.length - 40} more files`);
if (process.argv.includes('--strict') && total > 0) process.exit(1);
