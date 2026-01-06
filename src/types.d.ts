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
