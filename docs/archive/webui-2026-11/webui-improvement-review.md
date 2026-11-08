# Web UI correction and usability review

Implemented on `feature/webui-usability-20261108`, starting from clean `main` at `82aab82` (the accepted modularization merge). Implementation commit: `7c93925`. Verification date: 2026-11-08. This report describes implemented behavior and observed results; it is not a proposal for another frontend architecture.

## Original defects verified

- At 1440 × 900, computed Stats canvas height was **900 CSS pixels**, inherited from the global canvas rule. The Stats selector computed to **grid**, stacking its choices.
- Stats assigned its canvas width and height on every animation frame and reconstructed statistical summaries and the God Mode log. BrainViz retained its constructor rectangle after display resizing.
- Hall of Fame reconstructed its HTML on each active animation frame. Its integer-generation `% 1` condition provided no throttle. A disposable live-browser observation recorded **9 replacements in 1.5 seconds** with unchanged records; this is a mutation count under that test workload, not an application frame-rate benchmark.
- Graph pointer conversion independently scaled the element rectangle into the viewBox, disregarding the browser's aspect-ratio letterboxing. Node dragging captured a child that was subsequently replaced by rendering. Cancellation and expanded-editor Escape/focus behavior were incomplete.
- Join controls stayed in the keyboard flow behind an opacity-only hidden overlay. The nickname input removed its outline.
- The panel had a fixed width, expanded editing reserved a fixed sidebar strip, and advanced JSON actions could overflow at increased text size. A phone could not conveniently hide the panel.
- Graph Apply and Defaults reset immediately, contrary to the documented staged graph workflow. Blurring or randomizing the baseline seed also sent a reset. Accepted/rejected live replies refreshed unrelated controls and could erase reset drafts.
- Dynamically generated setting names were unassociated generic elements. Tabs had click behavior without keyboard/selection semantics. Saved-brain deletion had no confirmation.

## Implemented changes

### Rendering and bounded presentation

`styles.css` gives viewport dimensions only to the world canvas. Stats uses a deliberate 300 CSS-pixel height; neural visualization uses a responsive 260–400 CSS-pixel height. `canvas_surface.ts` matches buffers to CSS dimensions and device pixel ratio, skips hidden surfaces and avoids unchanged backing-buffer assignments. BrainViz has an explicit resize operation with margins for edge neurons.

Stats presentation now lives in `stats_view.ts`. It runs only when visible, at a maximum of four sidebar refreshes per second. History charts redraw when history, chart selection, dimensions or DPI change. Summary HTML and log nodes update only when their displayed content changes. The visualizer redraws when a new authoritative visualization payload or display geometry arrives. The packed world retains its animation-frame loop.

Hall of Fame refreshes on tab entry, authoritative server refresh, generation changes, incoming records and run replacement. Its registry keeps the existing stale-load guards. The view uses keyed rows and entry-ID callbacks rather than reconstructing buttons or dispatching through shifting array indexes. Unchanged records retain DOM and focus. Disconnected clients cannot issue resurrection requests.

Measured after correction, with the simulation continuing:

| Work observed over 1.5 seconds | Result |
| --- | ---: |
| Hall of Fame DOM mutations | 0 |
| Stats backing-buffer assignments | 0 |
| Unchanged God Mode log mutations | 0 |
| Live Stats summary updates | 5–6, bounded by the 4 Hz presentation cadence |

The canvas regression also verifies 120 unchanged sizing calls without another backing-buffer allocation. These observations establish reduced unnecessary work; they do not claim an overall engine or browser FPS improvement.

### Graph interaction and configuration workflow

`graph_diagram.ts` converts pointer coordinates with `getScreenCTM().inverse()`. Node/connection capture uses the stable SVG. Pointer cancellation, lost capture and window blur clear unfinished gestures. Explicitly loading another graph clears obsolete layout overrides. Diagram nodes/outputs support keyboard selection; the existing node/edge/output forms remain the alternative for structural editing and port assignment.

**Stage graph for reset** validates/stages the editor graph without changing the world. **Discard graph edits** restores the last staged graph or default stack, including its JSON representation. **Stage defaults**, baseline-seed edits and seed randomization also stage changes. **Apply and reset** is the one explicit same-seed population replacement action and is guarded against duplicate submissions. **New Run** continues to use the running configuration and a new seed.

A persistent pending notice remains above the protected configuration, with markers on edited collapsed groups. Live acknowledgements project only the admitted patch. Rejections restore live controls while retaining reset drafts. Invalid controls open their enclosing groups before browser validation requests focus. An uncertain reset/import outcome explicitly tells the user to reconnect and inspect the run before repeating the operation.

### Responsive layout, navigation and accessibility

`panel_navigation.ts` wires responsive collapse, tab roles/selection, roving tab stops, Left/Right and Home/End navigation, and sensible fallback when a capability-dependent tab disappears. It delegates activation to the existing tab handlers. Wider screens reserve canvas space for the panel; phone screens use an overlay and start collapsed. Rendering, camera fit and pointer conversion all use the same measured canvas dimensions. There is no second camera or simulation state.

The join overlay becomes inert and hidden to accessibility when dismissed; focus moves to the panel toggle. Modal Tab handling keeps join/expanded-editor focus in the visible workspace. Escape closes the expanded editor and restores its opener. Gameplay shortcuts do not intercept forms, tabs or modal editing.

Dynamic settings have stable IDs, associated labels, descriptions and conditional-availability explanations. Graph forms have accessible field names; Stats choices expose pressed state. Canvases have names and chart text summaries. Frequent statistics and selection telemetry do not use live announcements.

Camera fitting, view switching and checkpoint export remain available while settings are locked. Reset, import, checkpoint protection and generation-changing controls remain protected. Tabs retain the familiar functional organization, with direct links between visualizer and decision/statistical inspection. Persistent status shows connection/runtime facts, run prefix, generation/time and alive population from existing authoritative data.

### Artifacts and presentation

Saved-brain deletion requires a named, explicit confirmation; cancellation sends no delete or spawn. Names, metadata and provenance still use text nodes. Lists flag sensor differences early, while details explain compatibility and preserve exact identifiers with wrapping/scrolling. Hall of Fame champions and explicitly saved brains remain distinct. Existing pending/retry IDs, founder confirmation, cancellation semantics and native admission are preserved.

The visual pass keeps the dark arena, colorful snakes, mint/cyan/purple accents and dense tools. It adds consistent focus rings, readable text, surfaces, warnings, responsive button rows, an opaque expanded workspace and a usable adjacent/below inspector. Charts have compact large-value ticks, visible legends, generation identities/axis labels, stated quantities and an explanatory empty state. A small inline SVG favicon eliminates the otherwise missing-icon request.

## Validation performed

Environment: Windows x86_64, Node **24.19.0**, real production Rust addon, a fresh disposable SQLite/checkpoint instance, and Puppeteer's isolated Chromium **112.0.5614.0** profile. No owner database, config, checkpoint, saved brain or running experiment was opened for mutation. The server config, database and checkpoints were confined to ignored `.cache/webui-review/`.

| Check | Result |
| --- | --- |
| TypeScript `tsc -p tsconfig.json --pretty false` | Passed |
| ESLint `eslint .` | Passed |
| Production client `vite build` | Passed |
| Vitest unit project | **678 passed, 3 skipped**, 88 files passed |
| Vitest integration project | **221 passed, 3 skipped**, 20 files passed and 1 skipped |
| Focused native settings/reset and saved-brain suites | **11 passed**, 3 files passed |
| Disposable-browser regression harness | **83 checks passed**, zero page exceptions, console errors or failed HTTP responses |
| `git diff --check` | Passed |

The native suites were `server/rustServer.settings.native.test.ts`, `server/rustEngine/savedBrainSpawn.native.test.ts`, and `server/rustEngine/savedBrainCatalog.native.test.ts`. Their settings DOM stub was extended for label attributes; native production code was unchanged. Rust physics, inference, checkpoint formats and native ABI were not modified, so a new Rust build/full Rust suite was not required for this UI change.

Browser interactions included join focus and dismissal; tab arrow navigation; live/reset draft preservation; baseline-seed staging; invalid graph JSON; actual wide/tall node dragging and connection gestures in sidebar and expanded layouts; edge selection; pointer cancellation; keyboard graph selection; Escape; real Hall of Fame Spawn; single-submit native reset; native portable-brain fixture import and compatibility inspection; cancelled deletion; invalid duration inside a closed group; scrolling; collapse/expand; reconnection and spectator restoration. The test confirms that browsing/reconnecting sends no additional resets or New Run commands.

Layouts were captured at **1440 × 900**, **1280 × 720**, **1024 × 768**, **768 × 1024**, and **390 × 844**. Phone sizing also exercised **DPR 2**. A fresh touch-emulated 390 × 844 session verified initially collapsed controls, touch Spectate and tab navigation. Increased text sizing at **125%** checked wrapping and panel reachability.

The installed Edge **157.0.4341.42** was also attempted with a disposable headless profile, but exited before exposing an automation endpoint. It is **not** counted as a verified browser. Physical iOS/Android devices, Firefox and screen-reader software were not exercised.

## Review artifacts and reproduction

Screenshots and machine-readable assertions remain local generated artifacts, outside Git:

- `.cache/webui-review/before-join.png`, `before-stats.png`, `before-phone.png`: original behavior.
- `.cache/webui-review/screenshots/results.json`: browser checks, mutation counters and browser version.
- `desktop-join.png`, `desktop-locked.png`, `desktop-unlocked.png`, `desktop-hof.png`, `desktop-visualizer.png`: principal states.
- `desktop-stats-fitness.png`, `desktop-stats-diversity.png`, `desktop-stats-complexity.png`: populated history charts before the disposable reset.
- `editor-wide.png`, `editor-tall.png`: expanded graph gesture/inspector states.
- `brain-library-details.png`, `distillation.png`, `graph-validation-error.png`, `settings-validation-error.png`, `reconnecting.png`: artifacts, diagnostics and feedback.
- `{width}x{height}-{settings,stats,visualizer,editor,world}.png`: each requested viewport.
- `phone-touch-join.png`, `phone-touch-stats.png`, `text-scale-125.png`: touch and text-scaling states.

Run `npm run test:browser-ui -- --url=<disposable-frontend-url-with-server-query> --disposable` against an isolated fresh simulation. The explicit flag is mandatory because the harness resets that instance and imports a small existing native brain fixture. It reuses the Puppeteer/Chromium installation already supplied by Mermaid, adds no browser framework/dependency, writes under `.cache/webui-review/screenshots/`, and closes its own browser. `WEBUI_SCREENSHOTS` changes output location; `WEBUI_BROWSER` can select an available compatible executable. Stop the disposable server and Vite processes after testing; keep screenshots/results if needed for review.

## Preserved boundaries and limitations

Rust remains authoritative. Node still coordinates transport/persistence. No Rust source, simulation algorithm, protocol revision, checkpoint layout, database schema, optimizer/trainer or transaction sequencing changed. Archive transfers still use the original File/direct attachment; browser code does not decode or copy population internals. Most changes extend the specialized modules; `main.ts` remains the composition/state owner and is slightly shorter overall.

Pending indicators conservatively report edits; they are not a full structural diff/revert tracker. Reconnection and authoritative run replacement reload admitted settings/graphs, as before; export JSON or save a preset for a durable graph draft. Very dense graphs remain easier to inspect on a larger display; phone users can use the node/edge/output forms. Touch gameplay was deliberately not introduced. Charts provide current-history text summaries rather than a new hover/expanded-chart subsystem. No physical-device or assistive-technology conformance claim is made.

Future enhancements such as persistent configuration drafts across connections, diagram zoom/navigation and chart hover inspection are separate design work, not defects claimed fixed by this change.
