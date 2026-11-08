import { describe,expect,it } from 'vitest';
import { SUITE,createDescriptor,createFixture } from './checkpointPersistenceFixture.ts';

describe(SUITE, { timeout: 30_000 }, () => {

  it('terminates a worker after an invalid protocol response and makes close wait for that exit', async () => {
    const fixture = createFixture(
      new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url)
    );
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit(descriptor)).rejects.toThrow(/unknown response type/);
    await expect(fixture.client.close()).rejects.toThrow(/unknown response type/);
    expect(fixture.client.terminated).toBe(true);
  });

  it('rejects orphan cleanup and waits for termination after an invalid worker reply', async () => {
    const fixture = createFixture(new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url));
    await expect(fixture.client.reclaimManagedOrphans()).rejects.toThrow(/unknown response type/);
    await expect(fixture.client.close()).rejects.toThrow(/unknown response type/);
    expect(fixture.client.terminated).toBe(true);
  });

  it('terminates a worker after a correlated but mismatched acknowledgement', async () => {
    const fixture = createFixture(
      new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url),
      'mismatched'
    );
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit(descriptor)).rejects.toThrow(/acknowledgement mismatched/);
    await expect(fixture.client.close()).rejects.toThrow(/acknowledgement mismatched/);
    expect(fixture.client.terminated).toBe(true);
  });

  it('rejects pending work and close after an unexpected worker exit', async () => {
    const fixture = createFixture(
      new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url),
      'exit'
    );
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit(descriptor)).rejects.toThrow(/exited with code 3/);
    await expect(fixture.client.close()).rejects.toThrow(/exited with code 3/);
    expect(fixture.client.terminated).toBe(true);
  });

  it('treats exit zero during close as failure while a commit still awaits its reply', async () => {
    const fixture = createFixture(
      new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url),
      'exit-clean'
    );
    const descriptor = createDescriptor(fixture.managedRoot);
    const commit = fixture.client.commit(descriptor);
    const close = fixture.client.close();
    await expect(commit).rejects.toThrow(/exited cleanly with 1 pending operation/);
    await expect(close).rejects.toThrow(/exited cleanly with 1 pending operation/);
    expect(fixture.client.terminated).toBe(true);
  });

  it('terminates a persistence worker that stops making observable progress', async () => {
    const fixture = createFixture(
      new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url),
      'stall',
      100
    );
    await expect(fixture.client.commit(createDescriptor(fixture.managedRoot))).rejects.toThrow(
      /made no progress for 100 ms/
    );
    await expect(fixture.client.close()).rejects.toThrow(/made no progress/);
    expect(fixture.client.terminated).toBe(true);
  });

  it('terminates a persistence worker that stalls after starting an operation', async () => {
    const fixture = createFixture(
      new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url),
      'stall-after-progress',
      150
    );
    await expect(fixture.client.commit(createDescriptor(fixture.managedRoot))).rejects.toThrow(
      /made no progress for 150 ms/
    );
    await expect(fixture.client.close()).rejects.toThrow(/made no progress/);
    expect(fixture.client.terminated).toBe(true);
  });

  it('allows total worker duration to exceed the no-progress limit while counters advance', async () => {
    const fixture = createFixture(
      new URL('./checkpointPersistenceInvalidResponseWorker.ts', import.meta.url),
      'progressing',
      500
    );
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit(descriptor)).resolves.toMatchObject({
      operationId: descriptor.operationId,
      checkpointId: descriptor.logicalRootSha256
    });
  });
});
