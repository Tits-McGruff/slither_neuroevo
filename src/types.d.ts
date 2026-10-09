/// <reference types="vite/client" />

/** Allow importing JS modules via TypeScript. */
declare module '*.js';

/** Synchronous disposable interface for using declarations. */
interface Disposable {
  [Symbol.dispose](): void;
}

/** Async disposable interface for using declarations. */
interface AsyncDisposable {
  [Symbol.asyncDispose](): PromiseLike<void>;
}

/** Symbol constructor extensions for dispose symbols. */
interface SymbolConstructor {
  readonly dispose: unique symbol;
  readonly asyncDispose: unique symbol;
}

/** Slither build-time values supplied by Vite in development and production. */
interface ImportMetaEnv {
  readonly SLITHER_DEFAULT_WS_URL?: string;
  readonly SLITHER_SERVER_PORT?: number;
}
