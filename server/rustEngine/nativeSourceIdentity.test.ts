import { mkdirSync, mkdtempSync, renameSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it, type TestContext } from 'vitest';
import { computeNativeSourceIdentity } from './nativeSourceIdentity.ts';

/** Test-suite label used in runner output. */
const SUITE = 'native source identity';

/** Native crate directory resolved independently from the test working directory. */
const REAL_NATIVE_DIRECTORY = fileURLToPath(new URL('../../native', import.meta.url));

/** Temporary fixture roots removed after each test. */
const fixtureRoots: string[] = [];

/** Minimal fixed and Rust inputs for a valid build-script source manifest. */
const BASE_FIXTURE_FILES: Readonly<Record<string, string>> = {
  '.cargo/config.toml': '[build]\ntarget-dir = "target"\n',
  'Cargo.lock': 'version = 4\n',
  'Cargo.toml': '[package]\nname = "fixture"\n',
  'build.rs': 'fn main() {}\n',
  'package-lock.json': '{"lockfileVersion":3}\n',
  'package.json': '{"name":"fixture"}\n',
  'src/lib.rs': 'pub fn alpha() -> u32 { 1 }\n',
  'src/nested/zeta.rs': 'pub const ZETA: &str = "z";\n'
};

/**
 * Create a complete disposable native-manifest fixture.
 * @param overrides - Paths to add or replace after the base fixture.
 * @returns Absolute fixture root.
 */
function createFixture(overrides: Readonly<Record<string, string | Uint8Array>> = {}): string {
  const root = mkdtempSync(join(tmpdir(), 'slither-native-source-'));
  fixtureRoots.push(root);
  const files: Record<string, string | Uint8Array> = { ...BASE_FIXTURE_FILES, ...overrides };
  for (const [relativePath, content] of Object.entries(files)) {
    const absolutePath = resolve(root, ...relativePath.split('/'));
    mkdirSync(dirname(absolutePath), { recursive: true });
    writeFileSync(absolutePath, content);
  }
  return root;
}

/**
 * Create a symlink or visibly skip a platform that denies the operation.
 * @param context - Active Vitest context.
 * @param target - Existing target path.
 * @param linkPath - Link path to create.
 * @param type - File or directory-junction link kind.
 * @returns True when the link was created.
 */
function createSymlinkOrSkip(
  context: TestContext,
  target: string,
  linkPath: string,
  type: 'file' | 'junction'
): boolean {
  try {
    symlinkSync(target, linkPath, type);
    return true;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === 'EPERM' || code === 'EACCES' || code === 'ENOSYS') {
      console.warn(`[native source identity] skipping symlink check: ${code}`);
      context.skip();
    }
    throw error;
  }
}

afterEach(() => {
  for (const root of fixtureRoots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe(SUITE, () => {
  it('matches the retained source-SHA-v1 fixture vector and ordered evidence', () => {
    const fixture = createFixture();
    const identity = computeNativeSourceIdentity(fixture);

    expect(identity.sha256).toBe(
      '75080225674ab71ef480b89175c9e9d4ac6cd8d98afbdc6a1e86ea30bdf17cd8'
    );
    expect(identity.fileCount).toBe(8);
    expect(identity.manifest.map(entry => entry.relativePath)).toEqual([
      '.cargo/config.toml',
      'Cargo.lock',
      'Cargo.toml',
      'build.rs',
      'package-lock.json',
      'package.json',
      'src/lib.rs',
      'src/nested/zeta.rs'
    ]);
    expect(identity.manifest.every(entry => /^[0-9a-f]{64}$/.test(entry.canonicalSha256))).toBe(
      true
    );
    expect(identity.totalAccountedPathBytes).toBeGreaterThan(0);
  });

  it('treats CRLF and LF checkouts as the same logical text', () => {
    const lf = createFixture();
    const crlf = createFixture(
      Object.fromEntries(
        Object.entries(BASE_FIXTURE_FILES).map(([path, content]) => [
          path,
          content.replaceAll('\n', '\r\n')
        ])
      )
    );

    expect(computeNativeSourceIdentity(crlf).sha256).toBe(
      computeNativeSourceIdentity(lf).sha256
    );
  });

  it('sorts paths independently of creation order and frames file boundaries', () => {
    const first = createFixture({
      'src/z.rs': 'd',
      'src/a.rs': 'bc'
    });
    const sameFilesDifferentInsertionOrder = createFixture({
      'src/a.rs': 'bc',
      'src/z.rs': 'd'
    });
    const differentFraming = createFixture({
      'src/a.rs': 'b',
      'src/z.rs': 'cd'
    });

    const firstIdentity = computeNativeSourceIdentity(first);
    expect(computeNativeSourceIdentity(sameFilesDifferentInsertionOrder).sha256).toBe(
      firstIdentity.sha256
    );
    expect(computeNativeSourceIdentity(differentFraming).sha256).not.toBe(firstIdentity.sha256);
    expect(firstIdentity.manifest.findIndex(entry => entry.relativePath === 'src/a.rs')).toBeLessThan(
      firstIdentity.manifest.findIndex(entry => entry.relativePath === 'src/z.rs')
    );
  });

  it('sorts non-ASCII paths by raw UTF-8 bytes and matches Rust extension edge cases', () => {
    const fixture = createFixture({
      'src/é.rs': 'pub const COMPOSED: u8 = 1;\n',
      'src/é.rs': 'pub const DECOMPOSED: u8 = 2;\n',
      'src/.rs': 'this exact filename has no Rust extension',
      'src/dot..rs': 'pub const DOUBLE_DOT: u8 = 3;\n'
    });
    const paths = computeNativeSourceIdentity(fixture).manifest.map(entry => entry.relativePath);

    expect(paths.indexOf('src/é.rs')).toBeLessThan(paths.indexOf('src/é.rs'));
    expect(paths).not.toContain('src/.rs');
    expect(paths).toContain('src/dot..rs');
  });

  it('rejects low aggregate-path, entry, file, and total-content ceilings', () => {
    const fixture = createFixture();

    expect(() =>
      computeNativeSourceIdentity(fixture, { maxAggregatePathBytes: 1 })
    ).toThrow(/aggregate|path|ceiling/);
    expect(() => computeNativeSourceIdentity(fixture, { maxInspectedEntries: 1 })).toThrow(
      /entry safety ceiling/
    );
    expect(() => computeNativeSourceIdentity(fixture, { maxSourceFileBytes: 1 })).toThrow(
      /file ceiling/
    );
    expect(() => computeNativeSourceIdentity(fixture, { maxTotalSourceBytes: 1 })).toThrow(
      /canonical bytes.*ceiling/
    );
    expect(() => computeNativeSourceIdentity(fixture, { maxTotalSourceBytes: 0 })).toThrow(
      /positive safe integer/
    );
  });

  it('rejects invalid UTF-8, lone carriage returns, and embedded non-Rust inputs', () => {
    const invalidUtf8 = createFixture({ 'src/lib.rs': Uint8Array.of(0xff) });
    const loneCarriageReturn = createFixture({ 'src/lib.rs': 'pub fn bad() {}\rnext' });
    const embeddedInput = createFixture({
      'src/lib.rs': 'const DATA: &[u8] = include_bytes!("payload.bin");\n'
    });

    expect(() => computeNativeSourceIdentity(invalidUtf8)).toThrow(/not valid UTF-8/);
    expect(() => computeNativeSourceIdentity(loneCarriageReturn)).toThrow(
      /lone carriage return/
    );
    expect(() => computeNativeSourceIdentity(embeddedInput)).toThrow(/embeds a non-Rust file/);
  });

  it('rejects source-tree symlinks when the platform permits creating one', context => {
    const fixture = createFixture();
    if (
      !createSymlinkOrSkip(
        context,
        join(fixture, 'src', 'lib.rs'),
        join(fixture, 'src', 'linked.rs'),
        'file'
      )
    ) return;

    expect(() => computeNativeSourceIdentity(fixture)).toThrow(/refuses symlink/);
  });

  it('rejects fixed-input component symlinks when the platform permits creating one', context => {
    const fixture = createFixture();
    const realDirectory = join(fixture, 'cargo-real');
    renameSync(join(fixture, '.cargo'), realDirectory);
    if (!createSymlinkOrSkip(context, realDirectory, join(fixture, '.cargo'), 'junction')) return;

    expect(() => computeNativeSourceIdentity(fixture)).toThrow(/must not traverse a symlink/);
  });

  it('rejects a symlinked source root when the platform permits creating one', context => {
    const fixture = createFixture();
    const realDirectory = join(fixture, 'real-src');
    renameSync(join(fixture, 'src'), realDirectory);
    if (!createSymlinkOrSkip(context, realDirectory, join(fixture, 'src'), 'junction')) return;

    expect(() => computeNativeSourceIdentity(fixture)).toThrow(/must be one real directory/);
  });

  it.skipIf(process.platform === 'win32')(
    'accepts a POSIX filename containing backslash when Rust would select it',
    () => {
      const fixture = createFixture({ 'src/back\\slash.rs': 'pub const VALUE: u8 = 1;\n' });

      expect(computeNativeSourceIdentity(fixture).manifest.map(entry => entry.relativePath)).toContain(
        'src/back\\slash.rs'
      );
    }
  );

  it.skipIf(process.platform === 'win32')(
    'fails closed on invalid-UTF8 irrelevant POSIX entries with portable-tree wording',
    () => {
      const fixture = createFixture();
      const invalidPath = Buffer.concat([
        Buffer.from(join(fixture, 'src') + '/', 'utf8'),
        Buffer.from([0xff])
      ]);
      writeFileSync(invalidPath, 'ignored because it has no Rust extension');

      expect(() => computeNativeSourceIdentity(fixture)).toThrow(
        /unsupported portable native source tree:.*otherwise irrelevant/
      );
    }
  );

  it('hashes the current native tree without hard-coding its mutable digest', () => {
    const identity = computeNativeSourceIdentity(REAL_NATIVE_DIRECTORY);

    expect(identity.sha256).toMatch(/^[0-9a-f]{64}$/);
    expect(identity.fileCount).toBeGreaterThan(6);
    expect(identity.totalCanonicalBytes).toBeGreaterThan(0);
    expect(identity.manifest[0]?.relativePath).toBe('.cargo/config.toml');
  });
});
