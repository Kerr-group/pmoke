import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import remarkFrontmatter from 'remark-frontmatter';
import remarkMdx from 'remark-mdx';
import remarkParse from 'remark-parse';
import ts from 'typescript';
import { unified } from 'unified';

const root = path.resolve('content/docs/ja');
const ignoredAncestors = new Set([
  'blockquote',
  'code',
  'definition',
  'inlineCode',
  'inlineMath',
  'math',
  'mdxFlowExpression',
  'mdxJsxFlowElement',
  'mdxJsxTextElement',
  'mdxTextExpression',
  'yaml',
]);
const rules = [
  {
    name: 'polite or explanatory sentence ending',
    pattern: /(?:です|ます|でした|ました|ください|しましょう|できます|ありません)[。！？!?]?/u,
  },
  {
    name: 'generic translated call to action',
    pattern: /(?:ここをクリック|こちら(?:を|へ)?(?:クリック|参照|確認))/u,
  },
];

const files = await mdxFiles(root);
const failures = [];
for (const file of files) {
  const source = await readFile(file, 'utf8');
  const tree = unified()
    .use(remarkParse)
    .use(remarkFrontmatter, ['yaml'])
    .use(remarkMdx)
    .parse(source);
  walk(tree, [], (node, ancestors) => {
    if (node.type !== 'text' || ancestors.some((ancestor) => ignoredAncestors.has(ancestor.type))) {
      return;
    }
    for (const rule of rules) {
      if (rule.pattern.test(node.value)) {
        failures.push(
          `${path.relative(process.cwd(), file)}:${node.position?.start.line ?? 1}: ${rule.name}: ${node.value.trim()}`,
        );
      }
    }
  });
}

const uiFiles = (
  await Promise.all(
    ['app', 'components', 'lib'].map((directory) => typedSourceFiles(path.resolve(directory))),
  )
).flat();
for (const file of uiFiles) {
  const source = await readFile(file, 'utf8');
  const sourceFile = ts.createSourceFile(
    file,
    source,
    ts.ScriptTarget.Latest,
    true,
    file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
  visitTypeScript(sourceFile, sourceFile, file);
}

if (failures.length > 0) {
  throw new Error(`Japanese editorial lint failed:\n${failures.join('\n')}`);
}
console.log(
  `Japanese editorial lint passed for ${files.length} MDX and ${uiFiles.length} TypeScript files.`,
);

async function mdxFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map((entry) => {
      const target = path.join(directory, entry.name);
      return entry.isDirectory() ? mdxFiles(target) : entry.name.endsWith('.mdx') ? [target] : [];
    }),
  );
  return nested.flat().sort();
}

function walk(node, ancestors, visitor) {
  visitor(node, ancestors);
  if (!Array.isArray(node.children)) return;
  for (const child of node.children) walk(child, [...ancestors, node], visitor);
}

async function typedSourceFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map((entry) => {
      const target = path.join(directory, entry.name);
      return entry.isDirectory()
        ? typedSourceFiles(target)
        : /\.(?:ts|tsx)$/u.test(entry.name)
          ? [target]
          : [];
    }),
  );
  return nested.flat().sort();
}

function visitTypeScript(node, sourceFile, file) {
  let value;
  if (
    ts.isStringLiteral(node) ||
    ts.isNoSubstitutionTemplateLiteral(node) ||
    ts.isJsxText(node)
  ) {
    value = node.text;
  }
  if (value && /[\u3040-\u30ff\u3400-\u9fff]/u.test(value)) {
    for (const rule of rules) {
      if (rule.pattern.test(value)) {
        const position = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
        failures.push(
          `${path.relative(process.cwd(), file)}:${position.line + 1}: ${rule.name}: ${value.trim()}`,
        );
      }
    }
  }
  ts.forEachChild(node, (child) => visitTypeScript(child, sourceFile, file));
}
