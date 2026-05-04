# `defmt2log`

Keep writing real `defmt` in code that also runs on the host.

`defmt2log` lets a host binary decode those `defmt` frames and emit ordinary
[`log`](https://docs.rs/log) records, so you keep:

- full `defmt` format hints and syntax in the code
- `DEFMT_LOG` compile-time filtering
- normal host `log` tooling such as `RUST_LOG`, `env_logger`, and downstream
  `log` sinks

Initialization modes:

- `init_from_current_exe()` for the normal host-binary case
- `init_from_elf_path(path)` for any ELF path with a merged `.defmt` section,
  and also for the current executable
- `init_from_merged_elf_bytes(bytes)` for pre-merged ELF bytes

## Usage

```rust,no_run
# fn main() -> Result<(), Box<dyn std::error::Error>> {
env_logger::init();
defmt2log::init_from_current_exe()?;

defmt::info!("word {=u32:#010x}", 0x1234u32);
# Ok(())
# }
```

This example is `no_run` because rustdoc and libtest have special host-side
constraints:

- normal debug and release host binaries work
- plain libtest unit tests that emit `defmt` need a host `defmt` logger linked
  into that test binary, or they fail to link on `_defmt_acquire`,
  `_defmt_write`, and friends
- rustdoc doctest executables are worse: the bundled doctest rlib still
  contains `.defmt.info.*`, but the final `rust_out` test executable keeps only
  `.defmt.end` plus `_defmt_version_` / `_defmt_encoding_`, so there is no
  table to decode at runtime; those split metadata sections are dead-stripped
  from the final doctest executable

Use a normal host example or binary or spawn the host binary in an integration test.

Recommended default:

- build with `DEFMT_LOG=info`
- run with `RUST_LOG=info`
- leave `log` compile-time max-level features alone
- source locations are loaded on a best-effort basis; if they cannot be loaded,
  decoding still works and `defmt2log` warns once
- if `DEFMT_LOG` is unset or more restrictive than your callsites, normal
  `defmt::{trace,debug,info,warn,error}!` output is compiled out entirely; in
  that case `defmt2log` may still initialize successfully, but there is
  nothing for it to decode
- `DEFMT_LOG=off` and no `defmt::println!()` removes the need for a `defmt` `#[global_logger]`

## Filters

- `DEFMT_LOG` is the compile-time filter for `defmt`
- `RUST_LOG` is the runtime filter for the host `log` sink
- `max_level_*` and `release_max_level_*` affect ordinary host `log` callsites,
  not `defmt`

The important consequence is simple: `RUST_LOG` cannot bring back `defmt`
callsites that `DEFMT_LOG` compiled out.

## Avoid

- `DEFMT_LOG=trace` with `RUST_LOG=warn` unless you intentionally want to pay
  decode cost for logs the sink will hide
- using `max_level_*` features to control `defmt`
- using `init_from_merged_elf_bytes()` for a normal host executable; that API
  is only for ELFs that already contain a merged `.defmt` section. Normal host
  binaries should use `init_from_current_exe()`.
- expecting `init_from_elf_path(path)` to synthesize a table for an arbitrary
  non-running host binary without a merged `.defmt` section; that fallback is
  current-executable-only

## Limitations

- intended for normal host binaries first: no unittests, doctests yet
- it's typically less efficient than pure `log`; the overhead is
  the sum of the defmt overheads: serialization, deserialization, and formatting
- `init_from_current_exe()` is Linux-oriented today:
  the split-`.defmt.*` synthetic fallback depends on `/proc/self/maps`
- `init_from_current_exe()` is the normal Linux host path in both debug and
  release builds
- `init_from_elf_path()` and `init_from_merged_elf_bytes()` are the more
  portable modes: they work when the input already has a merged `.defmt`
  section
- native macOS current-executable support is not a supported path today
- host bitflags names require linker support that preserves `.defmt.end*`
  metadata; without that, host decoders only see the bitflags format tag and
  fall back to the raw numeric value
- a future host-side linker setup may preserve `.defmt.end*` and merge
  `.defmt.*` into one real `.defmt` section so `defmt-decoder::Table::parse()`
  can be used directly, but that is not a supported recipe yet
- every enabled `defmt` frame is decoded in-process

## Alternatives

- only `log::*`: simplest host setup, but you give up real `defmt` format
  hints and `DEFMT_LOG` compile-time filtering. That is a real loss for
  embedded-style diagnostics, and with `defmt2log` it is unnecessary.
- `defmt-or-log`: useful when one shared codebase must compile against either
  backend, but the shared callsites have to stay within the portable subset.
  If you want full `defmt` syntax and hints on host too, `defmt2log` is the
  better fit.
- `defmt-logger`: also aims at `defmt` -> `log`, but `defmt2log` is built
  around current `defmt-decoder` `log` interop and preserves the normal host
  `log` pipeline end to end: `RUST_LOG` filtering, existing sinks, and other
  downstream `log` machinery. It also avoids taking a dependency on the older
  `defmt-logger 0.1.0` / `defmt-decoder 0.1.x` stack.
- external decoding such as `defmt-print`: keeps full `defmt`, but moves the
  decode and process orchestration outside the program. `defmt2log` keeps the
  same logging stream inside the host process and feeds ordinary `log` sinks
  directly.
