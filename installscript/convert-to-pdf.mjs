// ============================================================================
//  convert-to-pdf.mjs  —  把 4 份 .md 安装说明转成 PDF
//  用法:  node convert-to-pdf.mjs
//  输出:  installscript\installation-docs.pdf  (合并)
//        installscript\*.pdf                  (单文件)
// ============================================================================
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';
import mdToPdfPkg from 'md-to-pdf';
const mdToPdf = mdToPdfPkg.mdToPdf || mdToPdfPkg.default || mdToPdfPkg;

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// ---- 要转换的文件（按顺序） ----
const files = [
  { src: 'PostgreSQL-PostGIS-Installation.zh-CN.md', out: '01-PostgreSQL-PostGIS-zh-CN.pdf',
    title: 'rust-gisserver · PostgreSQL + PostGIS 环境安装说明',
    subtitle: 'PostgreSQL 16.7 + PostGIS 3.5.2 · Windows x64 · 简体中文' },
  { src: 'PostgreSQL-PostGIS-Installation.en.md', out: '02-PostgreSQL-PostGIS-en.pdf',
    title: 'rust-gisserver · PostgreSQL + PostGIS Installation Guide',
    subtitle: 'PostgreSQL 16.7 + PostGIS 3.5.2 · Windows x64 · English' },
  { src: 'rust-gisserver-Setup-Installation.zh-CN.md', out: '03-rust-gisserver-Setup-zh-CN.pdf',
    title: 'rust-gisserver_Setup.exe 安装说明',
    subtitle: 'rust-gisserver v1.0.0 · NSSM 服务 · 简体中文' },
  { src: 'rust-gisserver-Setup-Installation.en.md', out: '04-rust-gisserver-Setup-en.pdf',
    title: 'rust-gisserver_Setup.exe Installation Guide',
    subtitle: 'rust-gisserver v1.0.0 · NSSM Service · English' },
];

// ---- 自定义 CSS：中文字体 + 代码块 + 表格 ----
const css = `
  @page {
    size: A4;
    margin: 18mm 16mm 22mm 16mm;
    @bottom-center { content: "rust-gisserver 安装文档 · 第 " counter(page) " 页 / 共 " counter(pages) " 页"; font-size: 9px; color: #888; font-family: "Microsoft YaHei", sans-serif; }
  }

  html, body {
    font-family: "Microsoft YaHei", "微软雅黑", "Segoe UI", sans-serif;
    font-size: 11pt;
    line-height: 1.75;
    color: #2d3748;
  }

  h1 { font-size: 20pt; color: #1a365d; border-bottom: 2px solid #2b6cb0; padding-bottom: 6px; margin-top: 28px; margin-bottom: 14px; page-break-after: avoid; }
  h2 { font-size: 15pt; color: #2b6cb0; border-left: 5px solid #2b6cb0; padding-left: 10px; margin-top: 22px; margin-bottom: 10px; page-break-after: avoid; }
  h3 { font-size: 12.5pt; color: #4a5568; margin-top: 16px; margin-bottom: 8px; page-break-after: avoid; }
  h4 { font-size: 11.5pt; color: #4a5568; margin-top: 12px; margin-bottom: 6px; page-break-after: avoid; }

  p { margin: 6px 0; }
  ul, ol { margin: 6px 0; padding-left: 28px; }
  li { margin: 2px 0; }
  strong { color: #1a365d; }
  code {
    font-family: "Consolas", "Courier New", monospace;
    font-size: 10pt;
    background: #edf2f7;
    color: #c53030;
    padding: 1px 6px;
    border-radius: 3px;
    border: 1px solid #e2e8f0;
  }

  pre {
    background: #1a202c;
    color: #e2e8f0;
    border-radius: 6px;
    padding: 14px 18px;
    font-family: "Consolas", "Courier New", monospace;
    font-size: 9.5pt;
    line-height: 1.55;
    overflow-wrap: break-word;
    white-space: pre-wrap;
    word-break: break-all;
    page-break-inside: avoid;
  }
  pre code {
    background: none;
    color: inherit;
    padding: 0;
    border: none;
    font-size: inherit;
  }

  table {
    border-collapse: collapse;
    width: 100%;
    margin: 10px 0 14px 0;
    font-size: 10pt;
    page-break-inside: avoid;
  }
  table th {
    background: #2b6cb0;
    color: white;
    padding: 8px 12px;
    border: 1px solid #2b6cb0;
    text-align: left;
    font-weight: 600;
  }
  table td {
    padding: 7px 12px;
    border: 1px solid #cbd5e0;
  }
  table tr:nth-child(even) td { background: #f7fafc; }

  blockquote {
    border-left: 4px solid #4299e1;
    background: #ebf8ff;
    padding: 10px 16px;
    margin: 12px 0;
    color: #2c5282;
    border-radius: 0 4px 4px 0;
  }

  hr { border: none; border-top: 1px dashed #cbd5e0; margin: 20px 0; }

  a { color: #2b6cb0; text-decoration: none; }

  em { color: #744210; }

  /* Cover page only (no header decoration) */
  .cover-title {
    font-size: 28pt; color: #1a365d; text-align: center;
    margin-top: 80px; margin-bottom: 10px;
  }
  .cover-subtitle {
    font-size: 14pt; color: #4a5568; text-align: center; margin-bottom: 30px;
  }
  .cover-rule { border-top: 3px solid #2b6cb0; width: 200px; margin: 0 auto 30px auto; }
  .cover-meta { text-align: center; color: #718096; font-size: 11pt; }
`;

// ---- 封面 HTML 模板 ----
function buildCoverMd(title, subtitle, index, total) {
  return `<!-- PDF_COVER_PAGE -->
# ${title}
<hr class="cover-rule">
*${subtitle}*
<br/><br/>

---
*文档 ${index} / ${total}*
*rust-gisserver Installation Documentation · 2026-11-20*
<br/><br/><br/>
---

`;
}

// ---- 主流程 ----
async function main() {
  console.log('📄 rust-gisserver 安装说明 → PDF');
  console.log(`共 ${files.length} 个文档待转换\n`);

  const pdfPaths = [];

  for (let i = 0; i < files.length; i++) {
    const f = files[i];
    const src = path.join(__dirname, f.src);
    const out = path.join(__dirname, f.out);

    console.log(`[${i + 1}/${files.length}] 转换 ${f.src} → ${f.out}`);

    // 读 markdown 内容
    let md = await fs.readFile(src, 'utf-8');

    // 在开头加 PDF 封面
    const cover = buildCoverMd(f.title, f.subtitle, i + 1, files.length);
    md = cover + md;

    // md-to-pdf 需要临时文件
    const tmpMd = path.join(__dirname, `_tmp_${i}.md`);
    await fs.writeFile(tmpMd, md, 'utf-8');

    try {
      const result = await mdToPdf(
        { path: tmpMd },
        {
          dest: out,
          css: css,
          pdf_options: {
            format: 'A4',
            printBackground: true,
            margin: { top: '18mm', bottom: '22mm', left: '16mm', right: '16mm' },
          },
          launch_options: { args: ['--no-sandbox'] },
        }
      );

      pdfPaths.push(out);
      const size = result?.buffer?.length ? result.buffer.length : (await fs.stat(out)).size;
      console.log(`    ✓ ${(size / 1024).toFixed(1)} KB`);
    } catch (err) {
      console.error(`    ✗ 失败: ${err.message}`);
      throw err;
    } finally {
      // 清理临时 md
      await fs.rm(tmpMd).catch(() => {});
    }
  }

  console.log('\n📦 合并所有 PDF...');
  await mergePdfs(pdfPaths, path.join(__dirname, 'installation-docs.pdf'));

  console.log('\n✅ 全部完成！产物：');
  for (const p of pdfPaths) {
    const size = (await fs.stat(p)).size;
    console.log(`   ${path.basename(p)}  (${(size / 1024).toFixed(1)} KB)`);
  }
  const mergedSize = (await fs.stat(path.join(__dirname, 'installation-docs.pdf'))).size;
  console.log(`   installation-docs.pdf  (${(mergedSize / 1024).toFixed(1)} KB)  ⭐`);
}

// ---- 合并所有 PDF（先尝试 md-to-pdf 内置，再用 pdf-lib 兜底）----
async function mergePdfs(inputPaths, outputPath) {
  try {
    // 尝试用 md-to-pdf 内置 merge
    if (mdToPdf.merge) {
      await mdToPdf.merge({ input: inputPaths, output: outputPath });
      return;
    }
    // 或从命名导出找
    const m = await import('md-to-pdf');
    if (m.merge) {
      await m.merge({ input: inputPaths, output: outputPath });
      return;
    }
  } catch (e) {
    console.log(`    md-to-pdf merge 不可用 (${e.message})，装 pdf-lib...`);
  }

  // 兜底：装 pdf-lib 然后手工合并
  console.log('    正在安装 pdf-lib ...');
  execSync('npm.cmd install pdf-lib --no-audit --no-fund', { cwd: __dirname, stdio: 'pipe' });
  const { PDFDocument } = await import('pdf-lib');
  const merged = await PDFDocument.create();
  for (const p of inputPaths) {
    const bytes = await fs.readFile(p);
    const doc = await PDFDocument.load(bytes);
    const pages = await merged.copyPages(doc, doc.getPageIndices());
    pages.forEach(page => merged.addPage(page));
  }
  const pdfBytes = await merged.save();
  await fs.writeFile(outputPath, pdfBytes);
}

main().catch(err => {
  console.error('\n❌ 转换失败:', err);
  process.exit(1);
});
