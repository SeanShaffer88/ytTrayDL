# Modular product direction

This document records the intended direction and current implementation.
The Downloads, video converter, lyrics/tags and player windows have optional direct-launch
desktop shortcuts; Windows runtime behavior still needs verification. The
converter supports sequential batches, video and audio profiles, trim and
basic transforms. Lyrics/tags supports local editing, opt-in online lyrics search,
and artwork extraction or replacement in supported formats. The local player
uses a separately installed ffplay and a queue that can optionally be saved
in app data; Windows behavior and actual playback still require validation.
ytTrayDL remains a Windows-first tray application; the existing downloader and
its settings continue to work while new capabilities are added incrementally.

## Product surfaces

- **Downloader:** the existing panel, download popup and queue remain available.
- **Video converter:** a separate window for converting local media, including
  output choices and a sequential batch queue.
- **Lyrics and tags:** a separate window for working with local audio metadata,
  artwork and lyrics. Existing lyrics must not be silently overwritten.
- **Player:** a separate window for music and video playback, with its own
  playback controls and queue. Online playback and account integration need
  their own service, privacy and dependency review.

Each implemented module must be reachable from the main panel, the tray menu
and its own Windows desktop shortcut. A shortcut must open the requested module
whether ytTrayDL is already running or not. Creating desktop shortcuts should
respect the user's installer choices; portable use must not silently create
shortcuts. Do not expose shortcuts or enabled menu entries for modules that do
not yet work.

## Architecture and integration

Keep one app instance and one tray icon. Module windows should use independent
viewports and state, so opening or closing one does not close another, stop
unrelated jobs or shut down the tray app. Preserve the existing recovery paths
for a hidden tray icon and the existing panel close behavior. Long-running
conversion, lookup and playback operations must not block the UI thread.

A launch intent should identify the requested module by a fixed, validated
identifier. On a cold start the primary instance opens that window. On a later
launch the request must be delivered to the existing instance and acknowledged;
the secondary process must report delivery failures rather than silently doing
nothing. The current show event cannot encode a module, while the existing
bounded `WM_COPYDATA` URL path illustrates the necessary single-instance
handoff. Module intents must be distinct from URLs and must not be interpreted
as shell commands. Repeated requests should focus the existing module window.

Module decision logic and persistence belong in testable library code; GUI
views collect intent and render state. Shared tools and process lifecycle
handling should be reused without coupling the modules' queues or settings.
The installer should add a shortcut only when its matching launch intent and
working module are shipped, and remove its own shortcuts on uninstall.

## Design and validation

Use the project's FluentGruv design standard across the main panel and module
windows, with consistent accessible controls, keyboard navigation, contrast,
window sizing and high-DPI behavior. Reconcile the current Command Deck theme
with that standard as part of implementation rather than assuming existing
styling is already compliant. Preserve the bounded, non-scrolling Settings
layout.

For every module, test library logic and cold-start/running-instance intent
routing, window lifecycle and shortcut creation/removal. Exercise actual UI,
tray, shortcuts, job behavior and multiple DPI settings on Windows before
claiming runtime verification. Follow the full repository gate in `AGENTS.md`.
