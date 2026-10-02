// Research-only executor: the locked donor queue adapter remains unmodified.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, mkdtempSync, readdirSync } from 'node:fs';
import { dirname, resolve, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { build } from 'esbuild';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../..');
const donor = resolve(root, 'research/commerce/donors/checkouts/apify--crawlee');
const commit = '438e3419626bd070f8984566bfb86ab9355f55d6';
const git = (...args) => execFileSync('git', ['-C', donor, ...args]);
assert.equal(git('rev-parse', 'HEAD').toString().trim(), commit);
const sourceEvidence = [];
function checked(path) {
  const raw = git('show', `${commit}:${path}`);
  assert.equal(readFileSync(resolve(donor, path), 'utf8').replaceAll('\r\n', '\n'), raw.toString('utf8').replaceAll('\r\n', '\n'), path);
  sourceEvidence.push({ path, commit_sha: commit, blob_hash: git('rev-parse', `${commit}:${path}`).toString().trim(), sha256: createHash('sha256').update(raw).digest('hex') });
  return raw.toString('utf8');
}
checked('pnpm-lock.yaml');
checked('LICENSE.md');
const outfile = resolve(here, 'node_modules/.cache/ecdev-crawlee-oracle.cjs');
mkdirSync(dirname(outfile), { recursive: true });
const result = await build({
  stdin: { contents: `export { FileSystemStorageBackend } from ${JSON.stringify(resolve(donor, 'packages/fs-storage/src/file-system-storage.ts'))};\nexport { RequestQueueBackend } from ${JSON.stringify(resolve(donor, 'packages/fs-storage/src/resource-clients/request-queue.ts'))};`, resolveDir: root, loader: 'ts' },
  outfile, bundle: true, platform: 'node', format: 'cjs', target: 'node22',
  nodePaths: [resolve(here, 'node_modules')], external: ['@crawlee/fs-storage-native', 'tough-cookie'], metafile: true, tsconfigRaw: { compilerOptions: { useDefineForClassFields: true } },
  plugins: [{ name: 'locked-symbol-reexports', setup(b) {
    // Select the actual imported donor exports without loading unrelated sitemap/browser utilities.
    // No validation, queue, ordering, persistence or clock implementation is replaced.
    b.onResolve({ filter: /^@crawlee\/utils\/internal$/ }, () => ({ path: 'queue-utils', namespace: 'donor-exports' }));
    b.onLoad({ filter: /.*/, namespace: 'donor-exports' }, () => ({ contents:
      `export { parseArgument } from ${JSON.stringify(resolve(donor, 'packages/utils/src/internals/validation.ts'))};\nexport * as schemas from ${JSON.stringify(resolve(donor, 'packages/utils/src/internals/schemas.ts'))};`, loader: 'ts', resolveDir: donor }));
    b.onResolve({ filter: /^@crawlee\/http-client$/ }, () => ({ path: resolve(donor, 'packages/http-client/src/base-http-client.ts') }));
  }}],
});
for (const input of Object.keys(result.metafile.inputs)) {
  const path = relative(donor, resolve(input)).replaceAll('\\', '/');
  if (!path.startsWith('..') && !path.includes('node_modules/')) checked(path);
}
const require = createRequire(import.meta.url);
const { FileSystemStorageBackend, RequestQueueBackend } = require(outfile);
const native = require('@crawlee/fs-storage-native');
if (process.argv.includes('--probe')) {
  console.log('native exports', Object.keys(native));
  const storage = new FileSystemStorageBackend({ localDataDirectory: resolve(root, 'target/crawlee-oracle/probe') });
  const queue = await storage.createRequestQueueBackend({ name: 'probe' });
  await queue.purge();
  console.log('queue methods', Object.getOwnPropertyNames(Object.getPrototypeOf(queue)));
  await queue.addBatchOfRequests([{ url: 'https://example.org/a', uniqueKey: 'a' }]);
  console.log('fetch', await queue.fetchNextRequest());
  console.log('metadata', await queue.getMetadata());
  await storage.teardown();
  console.log('compiled locked source modules', sourceEvidence.length);
  process.exit(0);
}
const lock = readFileSync(resolve(here, 'package-lock.json'), 'utf8');
const packageLock = JSON.parse(lock);
const backendIntegrity = packageLock.packages['node_modules/@crawlee/fs-storage-native'].integrity;
assert.ok(checked('pnpm-lock.yaml').includes(`integrity: ${backendIntegrity}`), 'Exact native backend is pinned by the donor lock');
const installedBinaries = readdirSync(resolve(here, 'node_modules/@crawlee')).filter(p => p.startsWith('fs-storage-native-'));
assert.equal(installedBinaries.length, 1, 'One actual platform binding must be installed');
const binaryPackage = `@crawlee/${installedBinaries[0]}`;
const binaryRecord = packageLock.packages[`node_modules/${binaryPackage}`];
assert.ok(checked('pnpm-lock.yaml').includes(`integrity: ${binaryRecord.integrity}`));
const binaryManifest = require(`${binaryPackage}/package.json`);
const binarySha256 = createHash('sha256').update(readFileSync(resolve(here, 'node_modules', binaryPackage, binaryManifest.main))).digest('hex');
const backendCheckout = resolve(root, 'research/commerce/donors/checkouts/apify--crawlee-storage');
const backendCommit = 'ac0c602c15dc16b2783a23d3e7137fbae183a34e';
assert.equal(execFileSync('git', ['-C', backendCheckout, 'rev-parse', 'HEAD']).toString().trim(), backendCommit);
const backendSourceEvidence = [];
for (const path of ['crawlee-storage/src/request_queue.rs', 'crawlee-storage/src/clock.rs', 'crawlee-storage-node/src/lib.rs', 'crawlee-storage-node/test/request_queue.test.ts', 'LICENSE.md']) {
  const raw = execFileSync('git', ['-C', backendCheckout, 'show', `${backendCommit}:${path}`]);
  assert.equal(readFileSync(resolve(backendCheckout, path), 'utf8').replaceAll('\r\n', '\n'), raw.toString('utf8').replaceAll('\r\n', '\n'));
  backendSourceEvidence.push({ path, commit_sha: backendCommit, blob_hash: execFileSync('git', ['-C', backendCheckout, 'rev-parse', `${backendCommit}:${path}`]).toString().trim(), sha256: createHash('sha256').update(raw).digest('hex') });
}
const a = (key, forefront = false) => ({ op: 'add', key, forefront });
const f = () => ({ op: 'fetch' });
const h = key => ({ op: 'handle', key });
const r = (key, forefront = false) => ({ op: 'reclaim', key, forefront });
const s = () => ({ op: 'status' });
const advance = millis => ({ op: 'advance', millis });
const reopen = () => ({ op: 'reopen' });
const cases = [
  ['empty', [s(), f(), s()]],
  ['deduplicate-pending', [a('a'), a('a'), a('b'), s(), f(), f(), f(), s()]],
  ['handled-remains-deduplicated', [a('a'), f(), h('a'), a('a'), f(), s()]],
  ['leased-not-fetched-twice', [a('a'), f(), f(), s(), h('a'), s()]],
  ['normal-fifo', [a('a'), a('b'), a('c'), f(), h('a'), f(), h('b'), f(), h('c'), s()]],
  ['forefront-lifo', [a('a'), a('b', true), a('c', true), a('d'), f(), h('c'), f(), h('b'), f(), h('a'), f(), h('d'), s()]],
  ['reclaim-tail', [a('a'), a('b'), f(), r('a'), f(), h('b'), f(), h('a'), s()]],
  ['reclaim-forefront', [a('a'), a('b'), f(), r('a', true), f(), h('a'), f(), h('b'), s()]],
  ['reclaim-forefront-over-existing-front', [a('a', true), a('b', true), f(), r('b', true), f(), h('b'), f(), h('a'), s()]],
  ['retry-through-reclaim', [a('a'), f(), r('a'), f(), r('a'), f(), h('a'), a('a'), f(), s()]],
  ['expiry-rejoins-tail', [a('a'), a('b'), f(), advance(200000), f(), h('b'), f(), h('a'), s()]],
  ['lease-expiry', [a('a'), f(), f(), advance(200000), f(), h('a'), s()]],
  ['reopen-handled-and-pending', [a('a'), a('b'), f(), h('a'), reopen(), a('a'), f(), h('b'), s()]],
  ['reopen-preserves-live-peer-lock', [a('a'), f(), reopen(), f(), advance(200000), f(), h('a'), s()]],
];
for (let mask = 0; mask < 32; mask++) {
  const steps = [];
  for (let i = 0; i < 5; i++) steps.push(a(`p${i}`, Boolean(mask & (1 << i))));
  steps.push(a('p2'), s());
  // Drain records without choosing the expected key in the harness.
  for (let i = 0; i < 6; i++) steps.push(f());
  steps.push(s());
  cases.push([`priority-permutation-${mask}`, steps]);
}
const outputCases = [];
const workspace = resolve(root, 'target/crawlee-oracle');
mkdirSync(workspace, { recursive: true });
for (const [name, steps] of cases) {
  const directory = mkdtempSync(resolve(workspace, 'queue-'));
  let client, queue;
  let elapsed = 0;
  async function open() {
    client = await native.FileSystemRequestQueueClient.open(undefined, 'oracle', undefined, directory, true, 'shared');
    client.advanceClockForTesting(elapsed);
    queue = await RequestQueueBackend.create({ name: 'oracle', cacheKey: 'oracle', nativeBackend: client });
    await queue.setExpectedRequestProcessingTimeSecs(180);
  }
  await open();
  const leases = new Map();
  const expected = [];
  for (const step of steps) {
    elapsed += 10; client.advanceClockForTesting(10);
    let result;
    switch (step.op) {
      case 'add': {
        const added = await queue.addBatchOfRequests([{ url: `https://queue.example/${step.key}`, uniqueKey: `https://queue.example/${step.key}` }], { forefront: step.forefront });
        const info = added.processedRequests[0];
        result = { added: !info.wasAlreadyPresent, handled: info.wasAlreadyHandled };
        break;
      }
      case 'fetch': {
        const request = await queue.fetchNextRequest();
        result = request?.url ?? null;
        if (request) leases.set(request.url, request);
        break;
      }
      case 'handle': result = Boolean(await queue.markRequestAsHandled(leases.get(`https://queue.example/${step.key}`))); break;
      case 'reclaim': result = Boolean(await queue.reclaimRequest(leases.get(`https://queue.example/${step.key}`), { forefront: step.forefront })); break;
      case 'advance': elapsed += step.millis; client.advanceClockForTesting(step.millis); result = null; break;
      case 'reopen': await open(); result = null; break;
      case 'status': {
        const info = await queue.getMetadata();
        result = { total: info.totalRequestCount, handled: info.handledRequestCount, pending: info.pendingRequestCount, empty: await queue.isEmpty(), finished: await queue.isFinished() };
        break;
      }
      default: throw new Error(`Unknown oracle operation ${step.op}`);
    }
    expected.push(result);
  }
  outputCases.push({ name, steps, expected });
}
const tests = ['test/core/storages/request_queue.test.ts', 'packages/fs-storage/test/request-queue/adapter.test.ts', 'packages/fs-storage/test/request-queue/reload-persistence.test.ts', 'packages/fs-storage/test/request-queue/request-queue-access.test.ts', 'packages/fs-storage/test/request-queue/prolong-request-lock.test.ts'];
for (const path of tests) checked(path);
const fixture = { donor: 'apify/crawlee', commit_sha: commit, oracle: 'LOCKED_REQUEST_QUEUE_BACKEND_WITH_PINNED_NATIVE_BINARY', source_evidence: [...new Map(sourceEvidence.map(e => [e.path, e])).values()], backend: { package: '@crawlee/fs-storage-native', version: '0.2.2', upstream: 'https://github.com/apify/crawlee-storage', published_git_head: backendCommit, integrity: backendIntegrity, platform: process.platform, arch: process.arch, platform_package: binaryPackage, platform_integrity: binaryRecord.integrity, binary_sha256: binarySha256, source_evidence: backendSourceEvidence, execution: 'PUBLISHED_INTEGRITY_PINNED_BINARY_NOT_LOCALLY_REBUILT' }, projection: 'Canonical HTTP GET URL is an explicit donor uniqueKey; compare added/handled flags, fetched URL and total/handled/pending/empty/finished. IDs, timestamps and internal orderNo are excluded. Native ECDEV keeps stricter token fencing and its own bounded retry/backoff policy.', contract: 'Deduplication, handled-state retention, leased exclusion, FIFO/forefront ordering, reclaim retry, expiration and shared reopen recovery; this is the queue backend, not the complete Crawlee crawler.', cases: outputCases };
const destination = resolve(root, 'domain/commerce/tests/fixtures/crawlee-frontier.json');
writeFileSync(destination, JSON.stringify(fixture, null, 2) + '\n', 'utf8');
writeFileSync(resolve(root, 'domain/commerce/tests/fixtures/crawlee-LICENSE.txt'), git('show', `${commit}:LICENSE.md`));
console.log(JSON.stringify({ status: 'DONOR_ORACLE_EXECUTED', cases: outputCases.length, operations: outputCases.reduce((n, c) => n + c.steps.length, 0), commit_sha: commit, backend_integrity: backendIntegrity }));
