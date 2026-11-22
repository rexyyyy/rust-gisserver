import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';

import { messages } from '../messages';

function flatten(value: unknown, prefix = ''): string[] {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    return [prefix];
  }
  return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) =>
    flatten(child, prefix ? `${prefix}.${key}` : key)
  );
}

const zhKeys = new Set(flatten(messages.zh_CN).filter(Boolean));
const enKeys = new Set(flatten(messages.en_US).filter(Boolean));

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) return walk(full);
    return /\.(vue|ts|tsx)$/.test(full) ? [full] : [];
  });
}

describe('i18n message catalogs', () => {
  it('zh_CN and en_US have identical keys', () => {
    const missingInEn = [...zhKeys].filter((key) => !enKeys.has(key)).sort();
    const missingInZh = [...enKeys].filter((key) => !zhKeys.has(key)).sort();
    expect({ missingInEn, missingInZh }).toEqual({ missingInEn: [], missingInZh: [] });
  });

  it('every i18n key referenced in src exists in the catalogs', () => {
    const srcRoot = resolve(process.cwd(), 'src');
    const knownPrefixes = /^(common|layout|map|system|monitor|misc|route|login|register|navbar)\./;
    const referenced = new Set<string>();
    for (const file of walk(srcRoot)) {
      const text = readFileSync(file, 'utf8');
      for (const match of text.matchAll(/(?:\$t|\bt)\(\s*['"]([^'"]+)['"]/g)) {
        // 'route.' + title in utils/i18n.ts is a deliberate dynamic lookup.
        if (knownPrefixes.test(match[1]) && !match[1].endsWith('.')) referenced.add(match[1]);
      }
    }
    const missing = [...referenced].filter((key) => !zhKeys.has(key) || !enKeys.has(key)).sort();
    expect(missing).toEqual([]);
  });
});
