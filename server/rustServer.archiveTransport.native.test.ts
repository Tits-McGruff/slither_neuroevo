/** Real HTTP archive framing and wire-limit acceptance with an unchanged prior experiment. */
import { readdir,stat,statfs } from 'node:fs/promises';
import { createServer,request,Server } from 'node:http';
import { join } from 'node:path';
import { expect,it } from 'vitest';
import { P0_ARCHIVE_UPLOAD_LIMIT } from './rustEngine/archiveUpload.ts';
import { ARCHIVE_FIXTURE_TIMEOUT_MS,bounded,experiment,exportAfterDisconnect,files,health,noTransferScratch,observeDisconnect,originalHttpEmit,preserved,raw,writeChunk } from './rustServerArchiveFixture.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';

describeNetworkSuite('Rust archive HTTP framing', () => {

  it('keeps the newer HTTP disconnect observer when an older fixture cleans up late', async () => {
    let requests = 0;
    const server = createServer((_incoming, response) => {
      requests++;
      response.writeHead(400, { 'Connection': 'close' });
      response.end('fixture import rejected');
    });
    let previous: ReturnType<typeof observeDisconnect> | undefined;
    let current: ReturnType<typeof observeDisconnect> | undefined;
    try {
      await new Promise<void>((done, reject) => {
        server.once('error', reject);
        server.listen(0, '127.0.0.1', () => { server.off('error', reject); done(); });
      });
      const address = server.address();
      if (!address || typeof address === 'string') throw new Error('observer fixture has no bound port');
      previous = observeDisconnect({ port: address.port });
      current = observeDisconnect({ port: address.port });
      previous.restore();
      previous.restore();
      const response = await fetch(`http://127.0.0.1:${address.port}/api/import/archive`, {
        method: 'POST', body: 'invalid', signal: AbortSignal.timeout(5000)
      });
      expect(response.status).toBe(400);
      expect(await response.text()).toBe('fixture import rejected');
      expect(await bounded(current.closed, 'newer observer lost the actual HTTP close')).toBe(false);
      expect(requests).toBe(1);
    } finally {
      previous?.restore(); current?.restore();
      await new Promise<void>((done, reject) => server.close(error => error ? reject(error) : done()));
    }
    expect(Server.prototype.emit).toBe(originalHttpEmit);
  });

  it.each([
    ['over-limit declared length', [`Content-Length: ${P0_ARCHIVE_UPLOAD_LIMIT + 1n}`], Buffer.alloc(0), false],
    ['noncanonical declared length', ['Content-Length: 01'], Buffer.from('x'), false],
    ['conflicting length and chunked framing', ['Content-Length: 1', 'Transfer-Encoding: chunked'], Buffer.from('0\r\n\r\n'), false],
    ['empty unframed upload', [], Buffer.alloc(0), false],
    ['truncated declared body', ['Content-Length: 100'], Buffer.from('incomplete'), true]
  ] as const)('preserves the experiment after %s', async (_name, headers, body, endInput) => {
    await experiment(async fixture => {
      const response = await raw(fixture, [...headers], body, endInput);
      expect(response).not.toContain('200 OK');
      if (!endInput) expect(response).toMatch(/^HTTP\/1\.1 400 /u);
      await exportAfterDisconnect(fixture);
      await preserved(fixture);
    });
  }, 20_000);

  it('rejects a valid archive followed by an extra byte beyond Content-Length before replacement', async () => {
    await experiment(async fixture => {
      const response = await raw(fixture, [`Content-Length: ${fixture.archive.byteLength}`],
        Buffer.concat([fixture.archive, Buffer.from('x')]));
      expect(response).not.toContain('200 OK');
      await preserved(fixture);
    });
  });

  it('imports a valid bounded chunked save without Content-Length', async () => {
    await experiment(async fixture => {
      const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' },
        body: (async function* () { yield fixture.archive; })() as unknown as BodyInit,
        duplex: 'half'
      } as RequestInit & { duplex: 'half' });
      expect(response.status).toBe(200);
      expect(await response.json()).toMatchObject({ ok: true, runId: fixture.identity['runId'] });
      await noTransferScratch(fixture.managedDirectory);
      expect(await files(fixture.managedDirectory)).toEqual(fixture.files);
      expect(await health(fixture.server)).toMatchObject({ ok: true, runId: fixture.identity['runId'] });
    });
  });

  it.runIf(process.env['SLITHER_FULL_UPLOAD_LIMIT_TEST'] === '1')(
    'rejects a chunked body crossing the actual four-GiB wire limit and removes its spool', async () => {
      await experiment(async fixture => {
        const space = await statfs(fixture.managedDirectory, { bigint: true });
        expect(space.bavail * space.bsize).toBeGreaterThan(12n * 1024n ** 3n);
        expect(P0_ARCHIVE_UPLOAD_LIMIT).toBe(4n * 1024n ** 3n);
        const client = request(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save',
            'Transfer-Encoding': 'chunked', 'Connection': 'close' }
        });
        fixture.requests.add(client);
        const terminal = new Promise<{ status: number; body: string }>((done, reject) => {
          client.once('error', reject);
          client.once('response', response => {
            const chunks: Buffer[] = [];
            response.on('data', bytes => chunks.push(bytes as Buffer));
            response.once('error', reject);
            response.once('end', () => done({ status: response.statusCode ?? 0,
              body: Buffer.concat(chunks).toString() }));
          });
        });
        void terminal.catch(() => {});
        const chunk = Buffer.alloc(1024 * 1024, 'x');
        const before = await health(fixture.server);
        const started = performance.now();
        let maximumSpoolBytes = 0;
        for (let written = 0n; written <= P0_ARCHIVE_UPLOAD_LIMIT; written += BigInt(chunk.byteLength)) {
          await writeChunk(client, chunk);
          if ((written + BigInt(chunk.byteLength)) % (64n * 1024n ** 2n) === 0n) {
            const partials = (await readdir(fixture.managedDirectory)).filter(name => name.endsWith('.upload.partial'));
            expect(partials).toHaveLength(1);
            maximumSpoolBytes = Math.max(maximumSpoolBytes,
              (await stat(join(fixture.managedDirectory, partials[0]!))).size);
          }
        }
        client.end();
        const result = await terminal;
        expect(result.status).toBe(400);
        expect(result.body).toContain(`archive upload exceeded the ${P0_ARCHIVE_UPLOAD_LIMIT}-byte limit`);
        expect(maximumSpoolBytes).toBeGreaterThan(Number(P0_ARCHIVE_UPLOAD_LIMIT) - 32 * 1024 ** 2);
        client.destroy();
        await preserved(fixture);
        const after = await health(fixture.server);
        expect(BigInt(`0x${after['completedStep'] as string}`))
          .toBeGreaterThan(BigInt(`0x${before['completedStep'] as string}`));
        console.log(JSON.stringify({ fullUploadLimitBytes: P0_ARCHIVE_UPLOAD_LIMIT.toString(),
          submittedBodyBytes: (P0_ARCHIVE_UPLOAD_LIMIT + BigInt(chunk.byteLength)).toString(),
          maximumSpoolBytes, wallMs: performance.now() - started, status: result.status }));
      });
    }, 180_000
  );
}, ARCHIVE_FIXTURE_TIMEOUT_MS);
