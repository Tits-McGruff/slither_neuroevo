import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';

/** Composition root must still route the selected file and download into the archive module. */
const ENTRY = ts.createSourceFile('main.ts', readFileSync(new URL('./main.ts', import.meta.url), 'utf8'),
  ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
/** Parsed implementation of the negotiated opaque Rust archive path. */
const SOURCE = ts.createSourceFile('archive_transport.ts', readFileSync(new URL('./archive_transport.ts', import.meta.url), 'utf8'),
  ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);

/** Resolve one named browser function without scanning its reference-only siblings. */
function browserFunction(name: string, source = SOURCE): ts.FunctionDeclaration {
  const found = source.statements.find(statement =>
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
    if (ts.isIfStatement(node) &&
        ts.isPropertyAccessExpression(node.expression) &&
        ts.isIdentifier(node.expression.expression) && node.expression.expression.text === 'ports' &&
        node.expression.name.text === capability) {
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
    expect(ENTRY.getText()).toMatch(/from\s+['"]\.\/archive_transport\.ts['"]/u);
    expect(browserFunction('exportServerSnapshot', ENTRY).body!.getText(ENTRY))
      .toMatch(/\bexportArchive\s*\(\s*getArchiveTransportPort\s*\(\s*\)\s*\)/u);
    const branch = archiveBranch(browserFunction('exportServerSnapshot'), 'serverArchiveExport');
    expect(branch).toContain('/api/export/latest');
    expect(branch).toMatch(/\.click\s*\(/u);
    expect(branch).not.toMatch(POPULATION_MATERIALIZATION);
  });

  it('keeps the negotiated import on the direct selected-File call graph', () => {
    expect(ENTRY.getText()).toMatch(/\bimportServerArchive\s*\(\s*e,\s*btnImport,\s*getArchiveTransportPort\s*\(\s*\)\s*\)/u);
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
