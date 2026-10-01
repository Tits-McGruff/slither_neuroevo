/* global window, document, requestAnimationFrame */
/**
 * Test-only browser observer, injected through tab-scoped CDP before joining the game.
 * In-app foreground captures require showing the browser host and calling Page.bringToFront.
 * Page visibility/focus flags alone do not prove that its compositor is in the foreground.
 */
(() => {
  /** Scalar counters and bounded timing samples; no frame/population payload is retained. */
  const state = {
    startedAtMs: performance.now(), frames: 0, frameIntervalsMs: [],
    animationIntervalsMs: [], renderIntervalsMs: [], renderDurationsMs: [],
    heapSamples: [], actions: 0, boostActions: 0, releaseActions: 0,
    assignments: 0, connections: 0, suppressedSensors: 0, suppressedFrames: 0,
    suppressSensors: false, suppressFrames: false, lastAction: null,
    visibilityChanges: [], lastFrameAtMs: null, lastAnimationAtMs: null,
    lastRenderAtMs: null
  };
  /** Original display scheduler; wrapping observes the real application callback. */
  const nativeAnimationFrame = window.requestAnimationFrame;
  /** Reuse callback wrappers without retaining application callbacks after their owner dies. */
  const animationWrappers = new WeakMap();
  window.requestAnimationFrame = function (callback) {
    if (callback === animation) return nativeAnimationFrame.call(window, callback);
    let wrapped = animationWrappers.get(callback);
    if (!wrapped) {
      wrapped = function (now) {
        if (state.lastRenderAtMs !== null && state.renderIntervalsMs.length < 100_000) {
          state.renderIntervalsMs.push(now - state.lastRenderAtMs);
        }
        state.lastRenderAtMs = now;
        const began = performance.now();
        try { callback(now); }
        finally {
          if (state.renderDurationsMs.length < 100_000) {
            state.renderDurationsMs.push(performance.now() - began);
          }
        }
      };
      animationWrappers.set(callback, wrapped);
    }
    return nativeAnimationFrame.call(window, wrapped);
  };
  /** Original browser socket implementation; every actual send still delegates to it. */
  const NativeSocket = window.WebSocket;
  /** Actual send implementation, retained before installing the observer. */
  const nativeSend = NativeSocket.prototype.send;
  /** Weak membership avoids retaining disconnected sockets or their payloads. */
  const observed = new WeakSet();
  /** Attach one capture listener to a new or existing application socket. */
  function observe(socket) {
    if (observed.has(socket)) return;
    observed.add(socket);
    state.connections += 1;
    socket.addEventListener('message', event => {
      if (typeof event.data !== 'string') {
        if (state.suppressFrames) {
          state.suppressedFrames += 1;
          event.stopImmediatePropagation();
          return;
        }
        const now = performance.now();
        if (state.lastFrameAtMs !== null && state.frameIntervalsMs.length < 100_000) {
          state.frameIntervalsMs.push(now - state.lastFrameAtMs);
        }
        state.lastFrameAtMs = now;
        state.frames += 1;
        return;
      }
      const message = JSON.parse(event.data);
      if (message.type === 'sensors' && state.suppressSensors) {
        state.suppressedSensors += 1;
        event.stopImmediatePropagation();
      } else if (message.type === 'assign') state.assignments += 1;
    }, { capture: true });
  }
  NativeSocket.prototype.send = function (data) {
    observe(this);
    if (typeof data === 'string') {
      const message = JSON.parse(data);
      if (message.type === 'action') {
        state.actions += 1;
        if (message.boost > 0) state.boostActions += 1;
        else state.releaseActions += 1;
        state.lastAction = { atMs: performance.now(), turn: message.turn, boost: message.boost, tick: message.tick };
      }
    }
    return nativeSend.call(this, data);
  };
  /** Socket wrapper that observes traffic and can suppress only test-selected inbound data. */
  class ObservedSocket extends NativeSocket {
    /** Attach one observer before the application installs its message callback. */
    constructor(...args) {
      super(...args);
      observe(this);
    }
  }
  window.WebSocket = ObservedSocket;
  window.__slitherAcceptance = state;
  /** Begin a bounded capture that can finish even if a diagnostic RPC times out. */
  state.startWindow = function (milliseconds) {
    if (!Number.isSafeInteger(milliseconds) || milliseconds < 5000 || milliseconds > 120_000) {
      throw new RangeError('browser capture must last from 5 to 120 seconds');
    }
    if (state.windowPending) throw new Error('browser capture is already running');
    state.windowPending = true;
    state.windowReport = null;
    state.frameIntervalsMs = [];
    state.animationIntervalsMs = [];
    state.renderIntervalsMs = [];
    state.renderDurationsMs = [];
    state.heapSamples = [];
    state.lastFrameAtMs = null;
    state.lastAnimationAtMs = null;
    state.lastRenderAtMs = null;
    const began = performance.now();
    const before = { frames: state.frames, actions: state.actions, assignments: state.assignments };
    window.setTimeout(() => {
      /** Summarize all intervals, including stalls and generation crossings. */
      function summarize(values) {
        const sorted = [...values].sort((a, b) => a - b);
        return { samples: sorted.length,
          p95Ms: sorted[Math.ceil(sorted.length * 0.95) - 1] ?? null,
          maxMs: sorted.at(-1) ?? null,
          fractionAtMost40Ms: sorted.length
            ? sorted.filter(value => value <= 40).length / sorted.length : null };
      }
      state.windowReport = {
        wallSeconds: (performance.now() - began) / 1000,
        frames: state.frames - before.frames, actions: state.actions - before.actions,
        assignments: state.assignments - before.assignments,
        animation: summarize(state.animationIntervalsMs),
        renderIntervals: summarize(state.renderIntervalsMs),
        renderDurations: summarize(state.renderDurationsMs),
        networkFrames: summarize(state.frameIntervalsMs),
        heapSamples: [...state.heapSamples],
        visibility: document.visibilityState, focus: document.hasFocus(),
        viewport: { width: window.innerWidth, height: window.innerHeight,
          devicePixelRatio: window.devicePixelRatio },
        userAgent: window.navigator.userAgent,
        canvas: [...document.querySelectorAll('canvas')].map(canvas => ({
          id: canvas.id, width: canvas.width, height: canvas.height,
          cssWidth: canvas.clientWidth, cssHeight: canvas.clientHeight })),
        visibilityChanges: state.visibilityChanges.filter(change => change.atMs >= began)
      };
      state.windowPending = false;
    }, milliseconds);
  };
  /** Coarse renderer heap observations supplement independent CDP heap measurements. */
  window.setInterval(() => {
    if (!performance.memory || state.heapSamples.length >= 600) return;
    state.heapSamples.push({ atMs: performance.now(),
      usedJSHeapBytes: performance.memory.usedJSHeapSize,
      totalJSHeapBytes: performance.memory.totalJSHeapSize });
  }, 1000);
  document.addEventListener('visibilitychange', () => {
    state.visibilityChanges.push({ atMs: performance.now(), visibility: document.visibilityState });
  });
  /** Observe rendering opportunities independently of incoming network frames. */
  function animation(now) {
    if (state.lastAnimationAtMs !== null && state.animationIntervalsMs.length < 100_000) {
      state.animationIntervalsMs.push(now - state.lastAnimationAtMs);
    }
    state.lastAnimationAtMs = now;
    requestAnimationFrame(animation);
  }
  requestAnimationFrame(animation);
})();
