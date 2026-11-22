// audit_i18n_coverage.js v3 — 最终版
// 运行: node manager/tools/audit_i18n_coverage.js
const fs = require('fs');
const path = require('path');

const BASE = path.resolve(__dirname, '..', 'backend');
const LOCALES_JSON = path.join(BASE, 'locales', 'en_US.json');
const SRC_DIR = path.join(BASE, 'src');

function loadI18n(jsonPath) {
    const data = JSON.parse(fs.readFileSync(jsonPath, 'utf-8'));
    const exactKeys = new Set(Object.keys(data.exact || {}));
    const prefixes = new Set((data.prefix || []).map(t => t[0]));
    return { exactKeys, prefixes };
}

const CHINESE_RE = /[\u4e00-\u9fff]/;
function hasChinese(s) { return CHINESE_RE.test(s); }

function extractFormatLiteral(fmtStr) {
    const m = fmtStr.match(/\{[^{}]*\}/);
    if (m) {
        const idx = fmtStr.indexOf(m[0]);
        const literal = fmtStr.substring(0, idx);
        if (hasChinese(literal)) return literal;
    }
    return null;
}

function extractChineseFromLine(line) {
    const results = [];
    const re = /"((?:[^"\\]|\\.)*)"/g;
    let m;
    while ((m = re.exec(line)) !== null) {
        let raw = m[1];
        let cleaned = raw.replace(/\\"/g, '"').replace(/\\\\/g, '\\').replace(/\\n/g, '\n');
        if (hasChinese(cleaned)) results.push(cleaned);
    }
    return results;
}

function findFormatMacroStrings(line) {
    const results = [];
    const re = /format!\s*\(\s*"((?:[^"\\]|\\.)*)"/g;
    let m;
    while ((m = re.exec(line)) !== null) {
        const fmtContent = m[1];
        if (hasChinese(fmtContent)) {
            const literal = extractFormatLiteral(fmtContent);
            results.push({ type: 'full', text: fmtContent });
            if (literal && literal !== fmtContent) {
                results.push({ type: 'prefix', text: literal });
            }
        }
    }
    return results;
}

function checkCoverage(text, exactKeys, prefixes) {
    if (exactKeys.has(text)) return 'EXACT';
    const sortedPrefixes = [...prefixes].sort((a, b) => b.length - a.length);
    for (const p of sortedPrefixes) {
        if (text.startsWith(p)) return 'PREFIX';
    }
    return 'UNCOVERED';
}

function walkRsFiles(dir) {
    const results = [];
    const stack = [dir];
    while (stack.length) {
        const cur = stack.pop();
        const entries = fs.readdirSync(cur, { withFileTypes: true });
        for (const e of entries) {
            const full = path.join(cur, e.name);
            if (e.isDirectory()) stack.push(full);
            else if (e.name.endsWith('.rs')) results.push(full);
        }
    }
    return results.sort();
}

// ============ 判断整行是否应跳过 ============
function shouldSkipLine(line, allLines, idx) {
    const trimmed = line.trim();

    // 1) 注释行（含 doc comment）
    if (trimmed.startsWith('//')) return { skip: true, reason: 'comment' };

    // 2) tracing / log / println / eprintln
    if (/tracing::\w+!\s*\(/.test(line)) return { skip: true, reason: 'tracing' };
    if (/(?:log::|eprintln!|println!|print!)/.test(line)) return { skip: true, reason: 'log' };

    // 3) 测试模块：向回找 #[cfg(test)] 或 mod tests，扩展搜索到 80 行
    for (let i = idx; i >= Math.max(0, idx - 80); i--) {
        if (/#\[cfg\(test\)\]/.test(allLines[i]) || /^\s*mod\s+tests\s*\{/.test(allLines[i])) {
            return { skip: true, reason: 'test' };
        }
    }

    return { skip: false, reason: '' };
}

// ============ 主扫描 ============
function scanRsFiles(srcDir, exactKeys, prefixes) {
    const exactlyCovered = [];
    const prefixCovered = [];
    const uncovered = [];
    const filteredOut = [];

    const rsFiles = walkRsFiles(srcDir);
    console.log(`共发现 ${rsFiles.length} 个 .rs 文件\n`);

    for (const rsFile of rsFiles) {
        const content = fs.readFileSync(rsFile, 'utf-8');
        const lines = content.split(/\r?\n/);
        const relPath = path.relative(BASE, rsFile).replace(/\\/g, '/');

        for (let lineno = 0; lineno < lines.length; lineno++) {
            const rawLine = lines[lineno];

            const { skip, reason } = shouldSkipLine(rawLine, lines, lineno);
            if (skip) continue;

            const chineseStrings = extractChineseFromLine(rawLine);
            const fmtStrings = findFormatMacroStrings(rawLine);

            const allChecks = [];
            for (const cs of chineseStrings) allChecks.push({ text: cs, type: 'literal' });
            for (const fs of fmtStrings) allChecks.push({ text: fs.text, type: fs.type });

            // 去重
            const seen = new Set();
            const unique = allChecks.filter(c => {
                const k = c.type + '|' + c.text;
                if (seen.has(k)) return false;
                seen.add(k);
                return true;
            });

            for (const { text, type } of unique) {
                const status = checkCoverage(text, exactKeys, prefixes);
                const entry = { file: relPath, line: lineno + 1, text, type };
                if (status === 'EXACT') exactlyCovered.push(entry);
                else if (status === 'PREFIX') prefixCovered.push(entry);
                else uncovered.push(entry);
            }
        }
    }

    return { exactlyCovered, prefixCovered, uncovered, filteredOut };
}

function sortEntries(arr) {
    return arr.sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line);
}
function groupByFile(arr) {
    const map = new Map();
    for (const e of arr) {
        if (!map.has(e.file)) map.set(e.file, []);
        map.get(e.file).push(e);
    }
    return map;
}

// ============ main ============
function main() {
    console.log('='.repeat(70));
    console.log('国际化覆盖率审计工具 v3（过滤注释/tracing/log/测试）');
    console.log('='.repeat(70));

    const { exactKeys, prefixes } = loadI18n(LOCALES_JSON);
    console.log(`已加载 en_US.json:`);
    console.log(`  - exact 键: ${exactKeys.size} 条`);
    console.log(`  - prefix:   ${prefixes.size} 条`);
    console.log();

    const { exactlyCovered, prefixCovered, uncovered } = scanRsFiles(SRC_DIR, exactKeys, prefixes);

    console.log('\n' + '='.repeat(70));
    console.log('结果汇总');
    console.log('='.repeat(70));
    console.log(`  ✅ EXACTLY_COVERED (精确匹配):   ${exactlyCovered.length} 处`);
    console.log(`  🔶 PREFIX_COVERED  (前缀匹配):   ${prefixCovered.length} 处`);
    console.log(`  ❌ UNCOVERED       (用户可见未覆盖): ${uncovered.length} 处`);
    console.log();

    // ============ UNCOVERED ============
    if (uncovered.length > 0) {
        // 去重（同一行同一文本不重复）
        const deduped = [];
        const seen = new Set();
        for (const e of sortEntries(uncovered)) {
            const k = e.file + ':' + e.line + '|' + e.text;
            if (seen.has(k)) continue;
            seen.add(k);
            deduped.push(e);
        }

        console.log('='.repeat(70));
        console.log(`❌ UNCOVERED — 用户可见但未被 exact 或 prefix 覆盖（${deduped.length} 处）`);
        console.log('='.repeat(70));

        const grouped = groupByFile(deduped);
        for (const [file, items] of [...grouped.entries()].sort()) {
            console.log(`\n  📄 ${file}  (${items.length} 处)`);
            for (const e of items) {
                const tag = e.type === 'prefix' ? '前缀片段' : '完整字符串';
                console.log(`     行 ${String(e.line).padStart(4)}  [${tag}]  ${e.text}`);
            }
        }

        console.log('\n' + '-'.repeat(70));
        console.log('📋 去重后的 exact 条目建议（可直接追加到 en_US.json 的 exact 对象中）:');
        console.log('-'.repeat(70));

        // 只取完整字符串（type=full），排除 prefix 片段
        const fullOnly = deduped.filter(e => e.type !== 'prefix');
        const uniqueTexts = [...new Set(fullOnly.map(e => e.text))];

        // 排除 prefix 片段中的冗余 — 如果完整串在 exact 中已有，则其 prefix 片段就是多余的
        const existingExactKeys = exactKeys;

        console.log('```json');
        for (const t of uniqueTexts) {
            const escaped = t.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n');
            console.log(`    "${escaped}": "",`);
        }
        console.log('```');
        console.log(`共需新增 ${uniqueTexts.length} 条 exact 条目。`);

        // prefix 片段需要分别处理：检查其完整串是否已存在
        const prefixOnly = deduped.filter(e => e.type === 'prefix');
        if (prefixOnly.length > 0) {
            console.log('\n' + '-'.repeat(70));
            console.log('⚠️  prefix 片段分析（这些是 format! 字符串中提取的字面量前缀）:');
            console.log('   如果对应的完整 format!("...{}...", var) 已在 exact 中覆盖，');
            console.log('   则这些短片段可以忽略；否则需要把完整字符串加到 exact。');
            console.log('-'.repeat(70));
            for (const e of prefixOnly) {
                console.log(`  ${e.file}:${e.line}  "${e.text.trimEnd()}"`);
            }
        }
    } else {
        console.log('✅ 所有用户可见中文字符串都已被 exact 或 prefix 覆盖！');
    }

    // ============ PREFIX_COVERED ============
    if (prefixCovered.length > 0) {
        console.log('\n' + '='.repeat(70));
        console.log(`🔶 参考: PREFIX_COVERED (这些通过 prefix 条目已覆盖)`);
        console.log('='.repeat(70));
        const shown = new Set();
        for (const e of sortEntries(prefixCovered)) {
            const trimmedKey = e.text.trimEnd();
            if (shown.has(trimmedKey)) continue;
            shown.add(trimmedKey);
            console.log(`  ${e.file}:${String(e.line).padStart(4)}  ${e.text}`);
        }
    }

    console.log('\n' + '='.repeat(70));
    console.log('审计完成');
    console.log('='.repeat(70));
}

main();
