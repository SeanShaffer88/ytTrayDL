# Process and completion-hook modules

For ytTrayDL 0.7.0, you can reuse `yttraydl::process` and
`yttraydl::hooks` from the existing MIT-licensed crate. These cohesive modules
keep process ownership and hook parsing outside the GUI. They are not separate
packages. Importing either module starts no processes, readers or background work.

## Process API

| API | Contract |
| --- | --- |
| `ProcessSpec::new(program).arg(value).args(values)` | Build a literal executable and argv vector. Set `current_dir` explicitly when needed. No shell parses arguments. |
| `ProcessLimits` | Supply a timeout, stdout/stderr byte limits and an unterminated-line byte limit. Defaults are 30 seconds, 8 MiB stdout, 256 KiB stderr and 1 MiB per line. |
| `CancellationToken::default()`, `cancel()`, `is_cancelled()` | Share cancellation through cloned tokens. Cancellation does not itself wait for cleanup. |
| `run_capture(&spec, limits, &token)` | Check cancellation before launching, then collect bounded output and clean up the owned tree. Return `ProcessOutput` on success. |
| `OwnedProcess::spawn(&spec)` | Launch immediately with containment; this lower-level call does not check a cancellation token before launch. |
| `OwnedProcess::capture(limits, &token)` | Consume the owner and capture bounded stdout/stderr. |
| `OwnedProcess::stream(limits, &token, callback)` | Consume the owner, deliver byte chunks and retain bounded diagnostic tails without a lifetime output cap. The line limit still applies. |
| `OwnedProcess::id()`, `try_wait()`, `terminate_and_wait()` | Identify the root, observe completion or explicitly terminate and reap the owned tree. |

You supply the executable, argv, operation deadline and cancellation token.
Captured output contains raw bytes. Streaming chunks can split UTF-8 characters
and lines; retain partial data in your decoder. Callbacks must return promptly:
they run on the supervising thread, so a blocking callback delays deadline and
cancellation handling. Do not assume ordering between stdout and stderr chunks.

`ProcessError` distinguishes `Spawn`, `Io`, `Timeout`, `Cancelled`,
`OutputLimit { stream }` and `Exit { code, stderr }`. Treat observer/reader errors,
callback panics and cleanup failures as errors, not successful cancellation.
Cleanup errors can take precedence over an earlier timeout or cancellation.
Use explicit termination when you need its result; dropping an owner attempts
termination and reaping but cannot return cleanup errors.

## Containment and limits

On Windows, the platform adapter creates the child suspended, assigns it to an
owned Job Object before execution and then resumes it. Containment failure fails
the launch. Cleanup targets the owned job, with kill-on-close semantics; it never
performs global process killing. Win32 calls remain in the platform adapter.

On Unix, the child starts in its own process group. Containment covers descendants
that stay in that group; a descendant that leaves it is outside this guarantee.
Capture supervision terminates leftover descendants when the root completes
naturally, rather than allowing inherited pipe handles to keep capture alive.
The supervisor continues checking cancellation and deadlines after both pipes
close. During cleanup it signals readers to stop, drops the output receiver to
unblock queue producers, and joins the readers. Pipe readers use interruptible
Windows reads or nonblocking Unix reads so inherited handles do not require EOF
to stop. Native Windows fixtures cover these cleanup paths, including termination
failure; Unix runtime verification remains pending. Other platforms fail launch
when containment is unavailable.

These are process ownership mechanisms, not security sandboxes. You remain
responsible for trusting the selected executable and its argument semantics.

## Hook API

`HookSpec::parse(template)` returns a literal `executable` and `args`, or an
actionable `String` error. Whitespace separates arguments outside double quotes;
double quotes group text and preserve empty arguments. Single quotes are literal.
An unmatched double quote fails. Backslashes are literal except before a double
quote: each pair becomes one backslash, and an odd remaining backslash escapes
the quote. Double a trailing backslash before a closing quote.

The executable cannot contain placeholders. Unquoted shell operators, scripts
and recognized interpreters are rejected. This validation is not an exhaustive
executable trust policy. `expand(file, dir, url, title)` consumes the parsed spec
and replaces `{file}`, `{dir}`, `{url}` and `{title}` once within each argument.
Inserted values are never tokenized or expanded again; unknown placeholders stay
literal. Parsing and expansion do not execute anything. Pass the resulting argv
to the process API only when execution is intended. See the
[hook migration guide](RELEASE-0.7.0.md#completion-hook-migration).

## Consumer example

You need Rust 1.89 or later and a local checkout or unpacked crate containing
these modules. Create a separate Cargo binary project and add this dependency to
its `Cargo.toml`, replacing the path with the actual checkout or unpacked crate:

```toml
[dependencies]
yttraydl = { path = "../ytTrayDL" }
```

Put this in the consumer's `src/main.rs`, then run `cargo run`. The example
intentionally cancels before capture, so it launches no executable:

```rust
use yttraydl::{
    hooks::HookSpec,
    process::{run_capture, CancellationToken, ProcessError, ProcessLimits, ProcessSpec},
};

fn main() -> Result<(), String> {
    let hook = HookSpec::parse(r#"receiver.exe "{file}" "{title}""#)?
        .expand("clip {title}.mp4", ".", "https://example.invalid/video", "雪");
    assert_eq!(hook.args, ["clip {title}.mp4", "雪"]);

    let spec = ProcessSpec::new(&hook.executable).args(&hook.args);
    let token = CancellationToken::default();
    token.cancel();
    assert!(matches!(
        run_capture(&spec, ProcessLimits::default(), &token),
        Err(ProcessError::Cancelled)
    ));
    Ok(())
}
```

A preliminary independent consumer passed against an unpacked `.crate`. Before
publishing, rebuild the final package and run this example in a separate project
against that unpacked archive; an earlier package does not verify later changes.
The hook-guard mutation check produced the expected test failure and the original
source hash was restored. This checks the regression guard, not executable trust.

## License and provenance

These modules are part of ytTrayDL under the repository's [MIT license](../LICENSE).
The implementation uses the Rust standard library, Unix `libc` process primitives
and Windows SDK bindings through the existing platform adapter. No additional
process-management package is required by this API.

The ProcessKit source reference in WinDevKit at commit
`0d698b65a81d8dd0efb73cf3d4186e594b852d36` (MIT) informed API ordering. This source
reference is provenance, not a runtime dependency or certified runtime evidence.
Verify consumption from the final unpacked crate in an independent project after
integration. Preserve the native test evidence and report remaining platform
runtime limits; see the [release contract](RELEASE-0.7.0.md).
