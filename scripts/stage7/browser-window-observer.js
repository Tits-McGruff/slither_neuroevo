/* global window, document, requestAnimationFrame */
/** Test-only browser observer, injected through tab-scoped CDP before joining the game. */
(() => {
  /** Scalar counters and bounded timing samples; no frame/population payload is retained. */
  const state = {
    startedAtMs: performance.now(), frames: 0, frameIntervalsMs: [],
    animationIntervalsMs: [], actions: 0, boostActions: 0, releaseActions: 0,
    assignments: 0, connections: 0, suppressedSensors: 0, suppressedFrames: 0,
    suppressSensors: false, suppressFrames: false, lastAction: null,
    visibilityChanges: [], lastFrameAtMs: null, lastAnimationAtMs: null
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
