/** Explicit location of verified release-asset inputs for historical claim audits. */
import { existsSync, readFileSync } from 'node:fs';
import { resolve as resolvePath, relative, isAbsolute, join } from 'node:path';

/** Repository whose compact catalog selects the immutable external payload. */
const repository = resolvePath(import.meta.dirname, '../..');

/** Resolve old logical evidence paths only inside an explicitly prepared cache. */
export function resolveEvidence(...parts: string[]): string {
  const original = resolvePath(...parts);
  const logical = relative(repository, original);
  if (isAbsolute(logical) || !logical.replaceAll('\\', '/').startsWith('docs/performance/')) return original;
  const catalog = JSON.parse(readFileSync(join(repository, 'docs/performance/catalog.json'), 'utf8')) as {
    artifact: {sha256: string};
  };
  if (!/^[0-9a-f]{64}$/u.test(catalog.artifact.sha256)) throw new Error('Invalid evidence catalog digest');
  const root = process.env['QUALIFICATION_EVIDENCE_ROOT']
    ?? join(repository, '.cache/qualification-evidence', catalog.artifact.sha256);
  if (!existsSync(join(root, 'verified.json'))) throw new Error(
    'Required historical evidence is unavailable. Run python PATH_TO_PYRL/pyrl_trainer/evidence_store.py '
    + 'fetch --catalog docs/performance/catalog.json --cache .cache/qualification-evidence. '
    + 'A missing release asset is a qualification failure.');
  return join(root, logical);
}
