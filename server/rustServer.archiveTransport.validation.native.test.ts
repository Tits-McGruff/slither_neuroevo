/** Real HTTP archive framing and wire-limit acceptance with an unchanged prior experiment. */
import { createHash } from 'node:crypto';
import { readFile,writeFile } from 'node:fs/promises';
import { dirname,join } from 'node:path';
import { zstdCompressSync } from 'node:zlib';
import { expect,it,vi } from 'vitest';
import WebSocket from 'ws';
import { compileGraph } from '../src/brains/graph/compiler.ts';
import * as archiveUpload from './rustEngine/archiveUpload.ts';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import type { ExperimentalRuntimeTelemetrySnapshot } from './rustEngine/runtimeTelemetry.ts';
import { advancing,alteredDecodedFrame,ARCHIVE_FIXTURE_TIMEOUT_MS,archiveEntries,archiveHeaderChecksum,conflictingArchive,controller,evolvedArchiveFixture,expansionBytes,expansionFrame,experiment,exportAfterDisconnect,headingChange,health,heldInput,orderingGraph,outcome,preserved,reversedConcatRole,semanticArchive,testCleanup } from './rustServerArchiveFixture.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';

describeNetworkSuite('Rust archive HTTP framing', () => {

  it.each([
    ['incompatible Concat input ordering', 'GRAPH_IDENTITY'],
    ['missing population record', 'INDEX_LENGTH'],
    ['missing dense population slot', 'INDEX_DENSE']
  ] as const)('rejects a correctly hashed save with %s before replacement', async (fault, code) => {
    await experiment(async fixture => {
      await evolvedArchiveFixture(fixture);
      const entries = archiveEntries(fixture.archive);
      const graph = compileGraph(orderingGraph());
      const reversed = compileGraph(orderingGraph(true));
      expect(reversed.totalParams).toBe(graph.totalParams);
      expect(reversed.key).not.toBe(graph.key);
      const roleIndex = code === 'GRAPH_IDENTITY' ? 1 : 2;
      const entry = entries[roleIndex]!;
      let role: Buffer = Buffer.from(fixture.archive.subarray(entry.data, entry.data + entry.size));
      if (code === 'GRAPH_IDENTITY') role = reversedConcatRole(role);
      else {
        const recordBytes = role.readUInt32LE(12);
        expect(recordBytes).toBe(104);
        expect(role.readBigUInt64LE(16)).toBe(12n);
        expect(role.byteLength).toBe(40 + 12 * recordBytes);
        if (code === 'INDEX_LENGTH') role = role.subarray(0, role.byteLength - recordBytes);
        else role.writeUInt32LE(0, 40 + recordBytes);
      }
      const damaged = semanticArchive(fixture.archive, roleIndex, role);
      const sourcePath = join(dirname(fixture.databasePath), 'user-original.save');
      await writeFile(sourcePath, fixture.archive, { flag: 'wx' });
      const sourceHash = createHash('sha256').update(fixture.archive).digest('hex');
      const peers = await Promise.all([controller(fixture, 'ui'), controller(fixture, 'bot')]);
      const before = await health(fixture.server);
      const activity = (before['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
      const baselines = peers.map(peer => ({ assignment: { ...peer.assignment! }, sample: peer.sample! }));
      const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
      const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
      const originalSpool = archiveUpload.spoolArchiveUpload;
      const spool = vi.spyOn(archiveUpload, 'spoolArchiveUpload').mockImplementationOnce(async options => {
        const result = await originalSpool(options);
        await heldInput(peers, false);
        return result;
      });
      const cleanup = testCleanup(() => { spool.mockRestore(); stage.mockRestore(); commit.mockRestore(); });
      try {
        const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', body: new Uint8Array(damaged), signal: AbortSignal.timeout(5000) });
        expect(response.status).toBe(400);
        expect(await response.json(), fault).toMatchObject({ ok: false,
          message: expect.stringContaining(`checkpoint ${code}:`) });
        expect(spool).toHaveBeenCalledOnce();
        expect(stage).toHaveBeenCalledOnce();
        expect(commit).not.toHaveBeenCalled();
        await preserved(fixture);
        await outcome(() => peers.every((peer, index) => peer.sample!.tick > baselines[index]!.sample.tick &&
          headingChange(baselines[index]!.sample, peer.sample!) < -0.01), 'steering during invalid import was lost');
        for (const [index, peer] of peers.entries()) {
          expect(peer.socket.readyState).toBe(WebSocket.OPEN);
          expect(peer.assignment).toEqual(baselines[index]!.assignment);
          expect(peer.packets.filter(packet => packet['type'] === 'assign')).toHaveLength(1);
          expect(peer.packets.filter(packet => ['error', 'stateReplaced'].includes(String(packet['type'])))).toEqual([]);
        }
        const after = await health(fixture.server);
        const applied = (after['telemetry'] as ExperimentalRuntimeTelemetrySnapshot).controllerActivity;
        expect(applied.player.appliedActions).toBe(activity.player.appliedActions + 1);
        expect(applied.trainer.appliedActions).toBe(activity.trainer.appliedActions + 1);
        expect(BigInt(`0x${after['completedStep'] as string}`)).toBeGreaterThan(BigInt(`0x${before['completedStep'] as string}`));
        expect(createHash('sha256').update(await readFile(sourcePath)).digest('hex')).toBe(sourceHash);
        expect(createHash('sha256').update(fixture.archive).digest('hex')).toBe(sourceHash);
        await exportAfterDisconnect(fixture);
        await preserved(fixture);
      } finally { cleanup(); }
    });
  }, 15_000);

  it.each(['nonzero role padding', 'nonzero second end block', 'hidden header name bytes',
    'continuous-file entry', 'symbolic-link entry', 'device entry', 'sparse entry', 'PAX entry',
    'duplicate role path', 'parent traversal path', 'missing role', 'unsupported save version',
    'corrupted logical root', 'false decoded byte count', 'corrupted graph entry'] as const)(
    'rejects %s in the uploaded save container before replacement', async fault => {
      await experiment(async fixture => {
        const entries = archiveEntries(fixture.archive);
        let damaged = Buffer.from(fixture.archive);
        if (fault === 'nonzero role padding') {
          const entry = entries.find(value => value.size % 512 !== 0)!;
          expect(entry).toBeDefined();
          damaged[entry.data + entry.size] = 1;
        } else if (fault === 'nonzero second end block') damaged[damaged.byteLength - 1] = 1;
        else if (fault === 'missing role') damaged = Buffer.concat([damaged.subarray(entries[0]!.end)]);
        else if (fault === 'corrupted graph entry') damaged[entries[1]!.data] = damaged[entries[1]!.data]! ^ 1;
        else if (fault === 'unsupported save version' || fault === 'corrupted logical root' ||
            fault === 'false decoded byte count') {
          const entry = entries[8]!;
          const manifest = JSON.parse(damaged.subarray(entry.data, entry.data + entry.size).toString()) as {
            archiveVersion: number; logicalRootSha256: string; roles: Array<{ decodedBytesHex: string }>;
          };
          if (fault === 'unsupported save version') manifest.archiveVersion = 5;
          else if (fault === 'corrupted logical root') {
            manifest.logicalRootSha256 = (manifest.logicalRootSha256.startsWith('0') ? '1' : '0') +
              manifest.logicalRootSha256.slice(1);
          } else manifest.roles[3]!.decodedBytesHex = 'ffffffffffffffff';
          const bytes = Buffer.from(JSON.stringify(manifest));
          expect(bytes.byteLength).toBe(entry.size);
          bytes.copy(damaged, entry.data);
        }
        else {
          const header = fault === 'duplicate role path' ? entries[1]!.header : entries[0]!.header;
          if (fault === 'hidden header name bytes') {
            const nul = damaged.indexOf(0, header);
            expect(nul).toBeLessThan(header + 99);
            damaged[nul + 1] = 0x78;
          } else if (fault === 'parent traversal path' || fault === 'duplicate role path') {
            const name = fault === 'parent traversal path' ? '../outside'
              : damaged.subarray(0, 100).toString().split('\0')[0]!;
            damaged.fill(0, header, header + 100);
            damaged.write(name, header, 'ascii');
          } else {
            const type = { 'continuous-file entry': '7', 'symbolic-link entry': '2',
              'device entry': '3', 'sparse entry': 'S', 'PAX entry': 'x' }[fault];
            damaged[header + 156] = type.charCodeAt(0);
          }
          archiveHeaderChecksum(damaged, header);
        }
        const originalHash = createHash('sha256').update(fixture.archive).digest('hex');
        const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' },
          body: damaged, signal: AbortSignal.timeout(5000)
        });
        expect(response.status, await response.text()).toBe(400);
        await preserved(fixture);
        expect(createHash('sha256').update(fixture.archive).digest('hex')).toBe(originalHash);
      });
    }
  );

  it.each([
    ['oversized decoder window', 'ZSTD_WINDOW'],
    ['hidden-size expansion stream', 'ZSTD_CONTENT_SIZE'],
    ['false frame output size', 'ZSTD_CONTENT_SIZE'],
    ['dictionary-dependent frame', 'ZSTD_FRAME'],
    ['oversized shuffled block', 'SHUFFLED_LIMIT'],
    ['oversized compressed block', 'SHUFFLED_LIMIT'],
    ['truncated frame header', 'ZSTD_FRAME'],
    ['extra compressed frame', 'ZSTD_FRAME'],
    ['changed decoded bits', 'LOGICAL_ROLE_SHA256']
  ] as const)('rejects %s inside a compressed population before replacement', async (fault, code) => {
    await experiment(async fixture => {
      const entries = archiveEntries(fixture.archive);
      const entry = entries[3]!;
      const manifest = JSON.parse(fixture.archive.subarray(entries[8]!.data,
        entries[8]!.data + entries[8]!.size).toString()) as { roles: Array<{ encoding: string }> };
      expect(manifest.roles[3]!.encoding).toBe('f32le-shuffle4-zstd-v1');
      expect(fixture.archive.subarray(entry.data, entry.data + 4).toString()).toBe('SFZ1');
      const floats = fixture.archive.readUInt32LE(entry.data + 4);
      const size = fixture.archive.readUInt32LE(entry.data + 8);
      expect(floats * 4).toBe(1024 * 1024);
      const originalFrame = fixture.archive.subarray(entry.data + 12, entry.data + 12 + size);
      const damaged = Buffer.from(fixture.archive);
      let replacement: Buffer | undefined;
      if (fault === 'oversized shuffled block') damaged.writeUInt32LE(floats + 1, entry.data + 4);
      else if (fault === 'oversized compressed block') damaged.writeUInt32LE(0xffff_ffff, entry.data + 8);
      else if (fault === 'truncated frame header') replacement = originalFrame.subarray(0, 3);
      else if (fault === 'extra compressed frame') replacement = Buffer.concat([
        originalFrame, zstdCompressSync(Buffer.alloc(4))
      ]);
      else if (fault === 'changed decoded bits') replacement = alteredDecodedFrame(originalFrame, floats * 4);
      else if (fault === 'hidden-size expansion stream') {
        replacement = expansionFrame();
        expect(replacement.byteLength).toBe(518);
        expect(await expansionBytes(replacement)).toBe(16 * 1024 * 1024);
      } else {
        const dictionary = fault === 'dictionary-dependent frame';
        replacement = Buffer.alloc(dictionary ? 11 : 10);
        Buffer.from([0x28, 0xb5, 0x2f, 0xfd, dictionary ? 0x81 : 0x80,
          fault === 'oversized decoder window' ? 0x58 : 0x50]).copy(replacement);
        if (dictionary) replacement[6] = 1;
        replacement.writeUInt32LE(floats * 4 + (fault === 'false frame output size' ? 1 : 0), dictionary ? 7 : 6);
      }
      if (replacement) {
        expect(replacement.byteLength + 12).toBeLessThan(entry.size);
        damaged.writeUInt32LE(replacement.byteLength, entry.data + 8);
        replacement.copy(damaged, entry.data + 12);
      }
      const sourceHash = createHash('sha256').update(fixture.archive).digest('hex');
      const stage = vi.spyOn(BackgroundOutputPump.prototype, 'stagePreparedImport');
      const commit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commitImport');
      try {
        const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
          method: 'POST', body: damaged, signal: AbortSignal.timeout(5000) });
        expect(response.status).toBe(400);
        expect(await response.json()).toMatchObject({ ok: false, message: expect.stringContaining(`checkpoint ${code}:`) });
        expect(stage).toHaveBeenCalledOnce();
        expect(commit).not.toHaveBeenCalled();
        await preserved(fixture);
        await advancing(fixture);
        expect(createHash('sha256').update(fixture.archive).digest('hex')).toBe(sourceHash);
      } finally { stage.mockRestore(); commit.mockRestore(); }
    });
  });

  it('rejects a valid same-run same-generation archive with different immutable content', async () => {
    await experiment(async fixture => {
      const body = await conflictingArchive(fixture);
      const originalHash = createHash('sha256').update(body).digest('hex');
      const response = await fetch(`http://127.0.0.1:${fixture.server.port}/api/import/archive`, {
        method: 'POST', body: new Uint8Array(body), signal: AbortSignal.timeout(5000) });
      expect(response.status, await response.clone().text()).toBe(400);
      expect(await response.json()).toMatchObject({ ok: false, message: expect.stringContaining('generation identity conflicts') });
      await preserved(fixture);
      await advancing(fixture);
      expect(createHash('sha256').update(body).digest('hex')).toBe(originalHash);
    });
  });
}, ARCHIVE_FIXTURE_TIMEOUT_MS);
