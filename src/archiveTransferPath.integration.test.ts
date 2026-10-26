import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';

/** Parsed browser entry source used to inspect only the negotiated Rust archive path. */
const SOURCE = ts.createSourceFile('main.ts', readFileSync(new URL('./main.ts', import.meta.url), 'utf8'),
  ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);

/** Resolve one named browser function without scanning its reference-only siblings. */
function browserFunction(name: string): ts.FunctionDeclaration {
  const found = SOURCE.statements.find(statement =>
    ts.isFunctionDeclaration(statement) && statement.name?.text === name);
  if (!found || !ts.isFunctionDeclaration(found) || !found.body) {
    throw new Error(`browser archive function ${name} is missing`);
  }
  return found;
}

/** Find the true branch selected by one production archive capability flag. */
function archiveBranch(root: ts.Node, capability: string): string {
  let branch: ts.Statement | undefined;
  const visit = (node: ts.Node): void => {
    if (ts.isIfStatement(node) && node.expression.getText(SOURCE) === capability) {
      branch = node.thenStatement;
      return;
    }
    ts.forEachChild(node, visit);
  };
  visit(root);
  if (!branch) throw new Error(`browser path does not select ${capability}`);
  return branch.getText(SOURCE);
}

/** Population-sized browser reads and rewrites forbidden in the direct save path. */
const POPULATION_MATERIALIZATION = /\b(?:new\s+)?(?:FileReader|Blob)\s*\(|\b(?:fetch|JSON\.stringify|Array\.from)\s*\(|\.(?:arrayBuffer|text|getReader|stream|readAsText|readAsArrayBuffer|json)\s*\(|\b(?:response|res|exportRes|saveRes)\.body\b/u;

describe('Rust archive browser transfer path', () => {
  it('recognizes population materialization without rejecting the direct anchor', () => {
    for (const forbidden of ['file.arrayBuffer()', 'file.stream()', 'reader.readAsText(file)',
      'response.body.getReader()', 'JSON.stringify(population)', 'new Blob(chunks)',
      'Array.from(weights)']) {
      expect(forbidden).toMatch(POPULATION_MATERIALIZATION);
    }
    expect('document.body.append(link)').not.toMatch(POPULATION_MATERIALIZATION);
  });

  it('keeps the negotiated export as a direct download link', () => {
    const branch = archiveBranch(browserFunction('exportServerSnapshot'), 'serverArchiveExport');
    expect(branch).toContain('/api/export/latest');
    expect(branch).toMatch(/\.click\s*\(/u);
    expect(branch).not.toMatch(POPULATION_MATERIALIZATION);
  });

  it('keeps the negotiated import on the direct selected-File call graph', () => {
    const branch = archiveBranch(SOURCE, 'serverArchiveImport');
    const upload = browserFunction('uploadServerArchive').body!.getText(SOURCE);
    expect(branch).toMatch(/uploadServerArchive\s*\(\s*file\b/u);
    expect(branch).not.toMatch(POPULATION_MATERIALIZATION);
    expect(upload).toMatch(/request\.send\s*\(\s*file\s*\)/u);
    expect(upload).toContain('JSON.parse(request.responseText)');
    expect([...upload.matchAll(/\bJSON\.parse\s*\(/gu)]).toHaveLength(1);
    expect(upload).not.toMatch(POPULATION_MATERIALIZATION);
  });
});
