# Settings Without Scrolling — 0.8.2

Settings must not introduce vertical or horizontal scrollbars, including choice
popups. This is a product constraint. The native window remains resizable;
related controls share pages when space permits, and smaller layouts split at
atomic control rows. Main and Downloads retain their existing scrolling behavior.

Initial, minimum and maximum client sizes are capped using the owned window's
actual monitor work area, native frame size and DPI. This capacity is cached on
opening and monitor changes; rendering does not query native geometry each frame.

The four tabs are General, Downloads, Rules and Tools. Section navigation jumps
to related controls; page navigation reaches every control. Compact windows omit
the redundant content title and non-Rules section selector. The native title-bar
close button and Alt+F4 still close Settings. No preference is changed by navigation.

Six rule collections keep independent page positions and stable item identities.
Draft text and chosen quality remain in memory when switching sections or pages.
At tight heights, quality pins and watched channels use explicit Browse/Add
pages. Successful additions return to the page containing the new entry; deleting
the last entry clamps the page position. Long text fields retain complete values
and support normal caret panning; tool paths provide full-value Copy.

Both tool operations retain phase, percent/bytes, speed/ETA and Cancel. Unknown
transfer values are shown explicitly. The Main progress display, canonical tool
provider, cancellation ownership and cached native geometry/status remain unchanged.

## Verification Contract

| Check | Required scope |
|---|---|
| Actual egui bounds | Every real Settings control/page, all conditional custom fields, missing tools and long paths/errors |
| Practical viewport/text matrix | 600×640 logical points at 100–200% text size; 480×420 at 100–150%; 400×350 at normal text; each with 0/1/2 operations |
| DPI simulation | Normal logical text at pixels-per-point 1, 1.25, 1.5 and 2; this is distinct from enlarging fonts again |
| Collection boundaries | All six lists with 0, 1, capacity, capacity+1 and 100 entries; actual navigation, add, delete-last and draft retention |
| Focus and state | Actual pointer-focused text editor remains focused and visible across resize/progress repacking; non-default settings serialize unchanged after navigation |
| Rust gates | Full locked tests, formatting, host/Windows/field clippy and checks, MSRV 1.89 and production release build |
| Native/package gates | Owned private-profile UI inspection and exact-head hosted CI; installer lifecycle executes only on hosted isolated Windows |

More extreme combinations of a tiny logical work area and doubled application
fonts are exercised separately as stress diagnostics. They are not described as
ordinary 200% OS DPI qualification. Headless rendering does not establish physical
monitor transitions, spoken screen-reader output or general accessibility compliance.
Native and hosted results are recorded with the exact candidate artifact; earlier
0.8.1 results are historical and are not silently promoted to this build.
