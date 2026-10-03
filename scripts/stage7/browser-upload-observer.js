/* global window, FileReader, XMLHttpRequest */
/** Observe browser upload metadata and file-reading calls without retaining any file bytes. */
(() => {
  /** Bounded scalar observations, owned only by this diagnostic page. */
  const state = { uploads: [], fileReads: [], restored: false };
  /** Original public methods and their installed wrappers, restored only if still ours. */
  const methods = [];
  /** Wrap a public API and delegate unchanged arguments to its original implementation. */
  function wrap(prototype, name, observe) {
    const original = prototype[name];
    if (typeof original !== 'function') return;
    const wrapped = function (...args) {
      observe(this, args);
      return Reflect.apply(original, this, args);
    };
    methods.push({ prototype, name, original, wrapped });
    prototype[name] = wrapped;
  }
  for (const name of ['arrayBuffer', 'text', 'bytes', 'stream']) {
    wrap(Blob.prototype, name, (blob) => {
      if (state.fileReads.length < 128) state.fileReads.push({ api: `Blob.${name}`, bytes: blob.size });
    });
  }
  for (const name of ['readAsArrayBuffer', 'readAsText', 'readAsDataURL', 'readAsBinaryString']) {
    wrap(FileReader.prototype, name, (_reader, args) => {
      if (state.fileReads.length < 128) state.fileReads.push({ api: `FileReader.${name}`, bytes: args[0]?.size ?? null });
    });
  }
  /** Weak request metadata never owns a file, request body or response text. */
  const requests = new WeakMap();
  wrap(XMLHttpRequest.prototype, 'open', (request, args) => {
    requests.set(request, { method: String(args[0]), url: String(args[1]) });
  });
  wrap(XMLHttpRequest.prototype, 'send', (request, args) => {
    const metadata = requests.get(request);
    if (!metadata || !metadata.url.includes('/api/import/archive') || state.uploads.length >= 8) return;
    const body = args[0];
    const upload = { ...metadata, bodyIsFile: body instanceof File,
      fileName: body instanceof File ? body.name : null,
      fileBytes: body instanceof File ? body.size : null,
      fileType: body instanceof File ? body.type : null,
      progressEvents: 0, progressBytes: 0, status: null, responseCharacters: null };
    state.uploads.push(upload);
    request.upload.addEventListener('progress', (event) => {
      upload.progressEvents++;
      upload.progressBytes = event.loaded;
    });
    request.addEventListener('loadend', () => {
      upload.status = request.status;
      if (request.responseType === '' || request.responseType === 'text') {
        upload.responseCharacters = request.responseText.length;
      }
    }, { once: true });
  });
  /** Remove this diagnostic's wrappers while preserving any subsequently installed owner. */
  function stop() {
    for (const { prototype, name, original, wrapped } of methods) {
      if (prototype[name] === wrapped) prototype[name] = original;
    }
    state.restored = true;
  }
  window.__slitherArchiveUploadObserver = { state, stop };
})();
