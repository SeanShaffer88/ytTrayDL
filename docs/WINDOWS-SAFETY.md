# Windows platform safety and validation

The product targets Windows 10/11. Native integration is isolated in
`src/platform/windows_impl.rs`; process and platform-neutral policy remains in
the library. The inline `SAFETY` comments document Win32 handle ownership,
callback lifetimes, buffer sizes, COM thread affinity and API preconditions.
These comments explain invariants, not a formal verification or proof against
all hostile processes at the same integrity level.

* A named mutex identifies an existing instance. Its process-owned handle is
  closed by `Platform::drop`. A waiter owns its event handle and exits after
  the stop signal; the message-window thread owns its window and registers and
  unregisters the clipboard listener and Ctrl+Alt+G hotkey on that thread.
* Clipboard reads open the clipboard with short bounded retries and close it
  after copying. `GlobalSize` bounds the UTF-16 slice, at most one million units,
  and a NUL terminator must occur inside that bound. Reads never mutate the
  clipboard. A failed read does not mark a sequence as consumed.
* `WM_COPYDATA` is copied during the callback after bounded payload validation;
  its receiver does not retain a sender pointer. The launching client checks the
  receiver's executable path and uses a bounded send. These measures do not
  authenticate every other process running as the same user.
* HWND caching checks the process, owning thread, and role property before use;
  handle values are borrowed and Win32 may reject a handle invalidated between
  calls. Geometry lookup stays on the owning GUI thread to avoid cross-thread
  text-message stalls.
* Taskbar COM state is thread-local, and an owned interface is released on its
  apartment thread. `CoUninitialize` balances only a successful initialization
  by this code. The explicitly indexed vtable slots must continue to match the
  documented `ITaskbarList3` ABI.
* External children are created suspended, assigned to an owned kill-on-close
  job, then resumed; assignment failure kills and waits for the root. On
  cancellation the job is terminated and acknowledgement is bounded. Pipe
  readers poll available bytes and stop rather than waiting forever for EOF.

Native Windows CI exercises process containment, clipboard allocation bounds,
window cache behavior and offline fake-downloader flows. Hosted VMs cannot
prove real tray/hotkey interactions, accessibility, GPU, multi-monitor DPI or
user-specific shell behavior; those remain separate manual Windows checks.
